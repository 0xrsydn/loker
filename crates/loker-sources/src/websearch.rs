use std::collections::{HashMap, HashSet};
use std::time::Duration;

use loker_core::board::{Ats, Board};
use serde_json::{Value, json};

use crate::discover::Candidates;
use crate::{HttpClient, post_json, segment, url};

const RESULTS_PER_QUERY: u32 = 100;
const TIMEOUT: Duration = Duration::from_secs(60);

pub fn board_hosts(ats: Ats) -> &'static [&'static str] {
    match ats {
        Ats::Ashby => &["jobs.ashbyhq.com"],
        Ats::Greenhouse => &[
            "boards.greenhouse.io",
            "job-boards.greenhouse.io",
            "boards.eu.greenhouse.io",
            "job-boards.eu.greenhouse.io",
        ],
        Ats::Lever => &["jobs.lever.co", "jobs.eu.lever.co"],
        Ats::Recruitee => &["recruitee.com"],
        Ats::SmartRecruiters => &["jobs.smartrecruiters.com", "careers.smartrecruiters.com"],
    }
}

fn ats_domain(ats: Ats) -> &'static str {
    match ats {
        Ats::Ashby => "ashbyhq.com",
        Ats::Greenhouse => "greenhouse.io",
        Ats::Lever => "lever.co",
        Ats::Recruitee => "recruitee.com",
        Ats::SmartRecruiters => "smartrecruiters.com",
    }
}

pub fn results_for(ats: Ats) -> u32 {
    let each = RESULTS_PER_QUERY / Ats::ALL.len() as u32;
    let rest = RESULTS_PER_QUERY % Ats::ALL.len() as u32;
    each + u32::from(ats == Ats::ALL[0]) * rest
}

pub const DEFAULT_QUERIES: [&str; 12] = [
    "software engineer",
    "machine learning engineer",
    "product designer",
    "data scientist",
    "account executive startup",
    "marketing manager startup",
    "customer success remote",
    "infrastructure engineer",
    "fintech startup",
    "healthcare technology company",
    "climate energy startup",
    "robotics hardware startup",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Exa,
    Parallel,
}

impl Engine {
    pub fn parse(s: &str) -> Option<Engine> {
        match s {
            "exa" => Some(Engine::Exa),
            "parallel" => Some(Engine::Parallel),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Engine::Exa => "exa",
            Engine::Parallel => "parallel",
        }
    }

    pub const ALL: [Engine; 2] = [Engine::Exa, Engine::Parallel];

    pub fn exclude_cap(self) -> u32 {
        match self {
            Engine::Exa => 1200,
            Engine::Parallel => 199,
        }
    }

    pub fn query_finder(self, query: &str) -> String {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        format!("{}:{}", self.name(), words.join(" "))
    }

    pub fn key_var(self) -> &'static str {
        match self {
            Engine::Exa => "EXA_API_KEY",
            Engine::Parallel => "PARALLEL_API_KEY",
        }
    }

    async fn search(
        self,
        client: &HttpClient,
        key: &str,
        query: &str,
        ats: Ats,
        exclude: &[String],
    ) -> anyhow::Result<(Vec<String>, Option<f64>)> {
        let (endpoint, body) = match self {
            Engine::Exa => (
                url("https://api.exa.ai", "/search"),
                json!({
                    "query": query,
                    "type": "auto",
                    "numResults": results_for(ats),
                    "includeDomains": board_hosts(ats),
                    "excludeDomains": exclude,
                }),
            ),
            Engine::Parallel => (
                url("https://api.parallel.ai", "/v1/search"),
                json!({
                    "objective": format!("Job postings on company boards hosted by {}: {query}", ats.as_str()),
                    "search_queries": [query],
                    "mode": "fast",
                    "advanced_settings": {
                        "max_results": results_for(ats),
                        "source_policy": {
                            "include_domains": [ats_domain(ats)],
                            "exclude_domains": exclude,
                        },
                    },
                }),
            ),
        };
        let resp = post_json(client, &endpoint, ("x-api-key", key), &body, TIMEOUT).await?;
        let urls = resp
            .get("results")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("{}: no results in the answer", self.name()))?
            .iter()
            .filter_map(|r| r.get("url")?.as_str().map(String::from))
            .collect();
        let cost = resp.pointer("/costDollars/total").and_then(Value::as_f64);
        Ok((urls, cost))
    }
}

pub fn board_path(board: &Board) -> String {
    let host = match board.ats {
        Ats::Ashby => "jobs.ashbyhq.com",
        Ats::Greenhouse => "job-boards.greenhouse.io",
        Ats::Lever => "jobs.lever.co",
        Ats::Recruitee => return format!("{}.recruitee.com", board.slug.to_lowercase()),
        Ats::SmartRecruiters => "jobs.smartrecruiters.com",
    };
    format!("{host}/{}", segment(&board.slug))
}

pub struct Searched {
    pub candidates: Candidates,
    pub ran: Vec<(String, u32)>,
    pub excluded: u32,
    pub cost_usd: Option<f64>,
    pub errors: Vec<String>,
}

pub async fn candidates(
    client: &HttpClient,
    engine: Engine,
    key: &str,
    queries: &[String],
    known: Vec<Board>,
) -> anyhow::Result<Searched> {
    let cap = engine.exclude_cap() as usize;
    let mut exclude: HashMap<Ats, Vec<Board>> = HashMap::new();
    for b in known {
        let list = exclude.entry(b.ats).or_default();
        if list.len() < cap {
            list.push(b);
        }
    }
    let excluded = exclude.values().map(Vec::len).sum::<usize>() as u32;
    let mut found: Vec<Option<Board>> = Vec::new();
    let mut ran = Vec::new();
    let mut cost_usd: Option<f64> = None;
    let mut errors = Vec::new();
    let mut answered = 0;
    for query in queries {
        let mut keys = HashSet::new();
        let mut failed = false;
        for ats in Ats::ALL {
            let paths: Vec<String> = exclude
                .get(&ats)
                .map(|list| list.iter().map(board_path).collect())
                .unwrap_or_default();
            match engine.search(client, key, query, ats, &paths).await {
                Ok((urls, cost)) => {
                    answered += 1;
                    let boards: Vec<Option<Board>> =
                        urls.iter().map(|u| Board::from_url(u)).collect();
                    for b in boards.iter().flatten() {
                        keys.insert(b.key());
                        let list = exclude.entry(b.ats).or_default();
                        if !list.iter().any(|e| e.key() == b.key()) {
                            list.insert(0, b.clone());
                            list.truncate(cap);
                        }
                    }
                    found.extend(boards);
                    if let Some(c) = cost {
                        cost_usd = Some(cost_usd.unwrap_or(0.0) + c);
                    }
                }
                Err(e) => {
                    failed = true;
                    errors.push(format!(
                        "{} {} \"{query}\": {e:#}",
                        engine.name(),
                        ats.as_str()
                    ));
                }
            }
        }
        if !failed {
            ran.push((query.clone(), keys.len() as u32));
        }
    }
    if answered == 0 && !errors.is_empty() {
        anyhow::bail!("{}", errors.join("; "));
    }
    Ok(Searched {
        candidates: Candidates::collect(engine.name().to_string(), found),
        ran,
        excluded,
        cost_usd,
        errors,
    })
}
