use std::collections::HashSet;
use std::time::Duration;

use loker_core::board::{Ats, Board};
use serde_json::{Value, json};

use crate::discover::Candidates;
use crate::{HttpClient, post_json, segment, url};

const RESULTS_PER_QUERY: u32 = 100;
const TIMEOUT: Duration = Duration::from_secs(60);

pub const BOARD_HOSTS: [&str; 7] = [
    "jobs.ashbyhq.com",
    "boards.greenhouse.io",
    "job-boards.greenhouse.io",
    "boards.eu.greenhouse.io",
    "job-boards.eu.greenhouse.io",
    "jobs.lever.co",
    "jobs.eu.lever.co",
];

const ATS_DOMAINS: [&str; 3] = ["ashbyhq.com", "greenhouse.io", "lever.co"];

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
            Engine::Parallel => 200 - ATS_DOMAINS.len() as u32,
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
        exclude: &[String],
    ) -> anyhow::Result<(Vec<String>, Option<f64>)> {
        let (endpoint, body) = match self {
            Engine::Exa => (
                url("https://api.exa.ai", "/search"),
                json!({
                    "query": query,
                    "type": "auto",
                    "numResults": RESULTS_PER_QUERY,
                    "includeDomains": BOARD_HOSTS,
                    "excludeDomains": exclude,
                }),
            ),
            Engine::Parallel => (
                url("https://api.parallel.ai", "/v1/search"),
                json!({
                    "objective": format!("Job postings on company boards hosted by Ashby, Greenhouse or Lever: {query}"),
                    "search_queries": [query],
                    "mode": "fast",
                    "advanced_settings": {
                        "max_results": RESULTS_PER_QUERY,
                        "source_policy": {
                            "include_domains": ATS_DOMAINS,
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
    let mut exclude: Vec<Board> = known;
    exclude.truncate(cap);
    let excluded = exclude.len() as u32;
    let mut found: Vec<Option<Board>> = Vec::new();
    let mut ran = Vec::new();
    let mut cost_usd: Option<f64> = None;
    let mut errors = Vec::new();
    for query in queries {
        let paths: Vec<String> = exclude.iter().map(board_path).collect();
        match engine.search(client, key, query, &paths).await {
            Ok((urls, cost)) => {
                let boards: Vec<Option<Board>> = urls.iter().map(|u| Board::from_url(u)).collect();
                let known: HashSet<String> = exclude.iter().map(Board::key).collect();
                let mut keys = HashSet::new();
                let mut fresh = Vec::new();
                for b in boards.iter().flatten() {
                    if keys.insert(b.key()) && !known.contains(&b.key()) {
                        fresh.push(b.clone());
                    }
                }
                ran.push((query.clone(), keys.len() as u32));
                exclude.splice(0..0, fresh);
                exclude.truncate(cap);
                found.extend(boards);
                if let Some(c) = cost {
                    cost_usd = Some(cost_usd.unwrap_or(0.0) + c);
                }
            }
            Err(e) => errors.push(format!("{} \"{query}\": {e:#}", engine.name())),
        }
    }
    if !errors.is_empty() && errors.len() == queries.len() {
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
