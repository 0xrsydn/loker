use std::collections::BTreeMap;
use std::path::Path;

use loker_core::api::{
    BoardTally, CONTRACT_VERSION, DiscoverParams, DiscoverReport, Error, Result,
};
use loker_core::board::{Ats, Board, BoardCheck, BoardStatus, Company};
use loker_core::normalize::decode_entities;
use loker_core::store::Store;
use serde_json::Value;

use crate::{
    HttpClient, Paced, Source, commoncrawl, get_json_unless_missing, get_text, joined, segment, url,
};

const ERRORS_SHOWN: usize = 10;

pub struct Candidates {
    pub finder: String,
    pub boards: Vec<Board>,
    pub unreadable: u32,
}

impl Candidates {
    pub fn collect(finder: String, found: impl IntoIterator<Item = Option<Board>>) -> Self {
        let mut by_key = BTreeMap::new();
        let mut unreadable = 0;
        for board in found {
            match board {
                Some(b) => {
                    by_key.entry(b.key()).or_insert(b);
                }
                None => unreadable += 1,
            }
        }
        Candidates {
            finder,
            boards: by_key.into_values().collect(),
            unreadable,
        }
    }
}

pub fn read_lines(finder: String, text: &str) -> Candidates {
    let lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    Candidates::collect(
        finder,
        lines.map(|l| Board::from_id(l).or_else(|| Board::from_url(l))),
    )
}

pub async fn discover(
    store: &Store,
    client: &HttpClient,
    p: DiscoverParams,
) -> Result<DiscoverReport> {
    let (candidates, skipped) = match p.from.as_str() {
        "commoncrawl" => {
            let crawl = match p.crawl {
                Some(c) if commoncrawl::valid_crawl(&c) => c,
                Some(c) => {
                    return Err(Error::Invalid(format!(
                        "`{c}` is not a crawl id (like CC-MAIN-2026-43)"
                    )));
                }
                None => commoncrawl::latest_crawl(client)
                    .await
                    .map_err(Error::Internal)?,
            };
            let finder = format!("commoncrawl:{crawl}");
            if !p.again && store.discovered(&finder).await? {
                (Candidates::collect(finder, []), true)
            } else {
                let found = commoncrawl::candidates(client, &crawl)
                    .await
                    .map_err(Error::Internal)?;
                (found, false)
            }
        }
        path => {
            for (set, field) in [(p.crawl.is_some(), "crawl"), (p.again, "again")] {
                if set {
                    return Err(Error::Invalid(format!(
                        "`{field}` only applies to commoncrawl"
                    )));
                }
            }
            let text = std::fs::read_to_string(path)
                .map_err(|e| Error::Invalid(format!("reading {path}: {e}")))?;
            let name = Path::new(path)
                .file_name()
                .map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
            (read_lines(format!("file:{name}"), &text), false)
        }
    };
    let new = if skipped {
        0
    } else {
        let new = store
            .add_boards(&candidates.finder, &candidates.boards)
            .await?;
        store
            .finish_discovery(&candidates.finder, candidates.boards.len() as u32)
            .await?;
        new
    };
    let (checked, errors) = check_unchecked(store, client, p.check).await?;
    Ok(DiscoverReport {
        contract_version: CONTRACT_VERSION,
        finder: candidates.finder,
        skipped,
        found: candidates.boards.len() as u32,
        new,
        unreadable: candidates.unreadable,
        checked,
        boards: tally(store).await?,
        errors,
    })
}

async fn check_unchecked(
    store: &Store,
    client: &HttpClient,
    limit: Option<u32>,
) -> Result<(BoardTally, Vec<String>)> {
    let boards = store
        .boards_with_status(BoardStatus::Unchecked, limit)
        .await?;
    let mut checked = BoardTally::default();
    let mut errors = Vec::new();
    let mut checks = Paced::start(
        boards.clone(),
        |b| Source::from(b).host(),
        |b| {
            let client = client.clone();
            async move { check_board(&client, &b).await.map_err(Error::Internal) }
        },
    );
    while let Some((i, done)) = checks.next().await {
        let board = &boards[i];
        match joined(done) {
            Ok(check) => {
                store.record_board_check(board, Ok(&check)).await?;
                checked.add(check.status, 1);
            }
            Err(e) => {
                let message = e.to_string();
                store.record_board_check(board, Err(&message)).await?;
                checked.failed += 1;
                if errors.len() < ERRORS_SHOWN {
                    errors.push(format!("{board}: {message}"));
                }
            }
        }
    }
    Ok((checked, errors))
}

async fn tally(store: &Store) -> Result<BoardTally> {
    let mut boards = BoardTally::default();
    for (status, failed, n) in store.board_counts().await? {
        boards.add(status, n);
        if failed && status == BoardStatus::Unchecked {
            boards.failed += n;
        }
    }
    Ok(boards)
}

pub async fn check_board(client: &HttpClient, board: &Board) -> anyhow::Result<BoardCheck> {
    let slug = segment(&board.slug);
    let status = |live: bool| {
        if live {
            BoardStatus::Live
        } else {
            BoardStatus::Empty
        }
    };
    let missing = BoardCheck {
        slug: board.slug.clone(),
        status: BoardStatus::Missing,
        jobs: None,
        company: None,
    };
    match board.ats {
        Ats::Ashby => {
            let api = url(
                "https://api.ashbyhq.com",
                &format!("/posting-api/job-board/{slug}"),
            );
            let Some(body) = get_json_unless_missing(client, &api).await? else {
                return Ok(missing);
            };
            let jobs: Vec<&Value> = body
                .get("jobs")
                .and_then(Value::as_array)
                .map(|all| {
                    all.iter()
                        .filter(|j| j.get("isListed").and_then(Value::as_bool) != Some(false))
                        .collect()
                })
                .unwrap_or_default();
            let spelled = jobs
                .iter()
                .filter_map(|j| j.get("jobUrl")?.as_str())
                .filter_map(Board::from_url)
                .find(|b| b.key() == board.key())
                .map_or_else(|| board.slug.clone(), |b| b.slug);
            let company = if jobs.is_empty() {
                None
            } else {
                let page = url(
                    "https://jobs.ashbyhq.com",
                    &format!("/{}", segment(&spelled)),
                );
                page_company(client, &page, ashby_organization).await
            };
            Ok(BoardCheck {
                slug: spelled,
                status: status(!jobs.is_empty()),
                jobs: Some(jobs.len() as u32),
                company,
            })
        }
        Ats::Greenhouse => {
            let api = url(
                "https://boards-api.greenhouse.io",
                &format!("/v1/boards/{slug}/jobs"),
            );
            let Some(body) = get_json_unless_missing(client, &api).await? else {
                return Ok(missing);
            };
            let jobs = body
                .get("jobs")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let company = jobs
                .iter()
                .find_map(|j| j.get("company_name")?.as_str())
                .map(|name| decode_entities(name.trim()))
                .filter(|name| !name.is_empty())
                .map(|name| Company {
                    name,
                    website: None,
                });
            Ok(BoardCheck {
                slug: board.slug.to_lowercase(),
                status: status(!jobs.is_empty()),
                jobs: Some(jobs.len() as u32),
                company,
            })
        }
        Ats::Lever => {
            let api = url(
                "https://api.lever.co",
                &format!("/v0/postings/{slug}?mode=json&limit=1"),
            );
            let Some(body) = get_json_unless_missing(client, &api).await? else {
                return Ok(missing);
            };
            let live = body.as_array().is_some_and(|a| !a.is_empty());
            let company = if live {
                let page = url("https://jobs.lever.co", &format!("/{slug}"));
                page_company(client, &page, page_title).await
            } else {
                None
            };
            Ok(BoardCheck {
                slug: board.slug.clone(),
                status: status(live),
                jobs: None,
                company,
            })
        }
    }
}

async fn page_company(
    client: &HttpClient,
    page: &str,
    read: fn(&str) -> Option<Company>,
) -> Option<Company> {
    match get_text(client, page).await {
        Ok(html) => read(&html),
        Err(e) => {
            tracing::warn!(%page, "no company name from the board page: {e}");
            None
        }
    }
}

pub fn ashby_organization(html: &str) -> Option<Company> {
    let marker = "window.__appData = ";
    let data = &html[html.find(marker)? + marker.len()..];
    let app: Value = serde_json::Deserializer::from_str(data)
        .into_iter()
        .next()?
        .ok()?;
    let org = app.get("organization")?;
    let name = org.get("name")?.as_str()?.trim();
    let website = org
        .get("publicWebsite")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|w| w.starts_with("http"));
    (!name.is_empty()).then(|| Company {
        name: name.to_string(),
        website: website.map(String::from),
    })
}

pub fn page_title(html: &str) -> Option<Company> {
    let start = html.find("<title>")? + "<title>".len();
    let end = start + html[start..].find("</title>")?;
    let name = decode_entities(html[start..end].trim());
    (!name.is_empty()).then_some(Company {
        name,
        website: None,
    })
}
