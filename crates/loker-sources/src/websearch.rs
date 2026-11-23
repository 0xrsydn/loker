use std::time::Duration;

use loker_core::board::Board;
use serde_json::{Value, json};

use crate::discover::Candidates;
use crate::{HttpClient, post_json, url};

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
    ) -> anyhow::Result<(Vec<String>, Option<f64>)> {
        let (endpoint, body) = match self {
            Engine::Exa => (
                url("https://api.exa.ai", "/search"),
                json!({
                    "query": query,
                    "type": "auto",
                    "numResults": RESULTS_PER_QUERY,
                    "includeDomains": BOARD_HOSTS,
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
                        "source_policy": { "include_domains": ATS_DOMAINS },
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

pub struct Searched {
    pub candidates: Candidates,
    pub cost_usd: Option<f64>,
    pub errors: Vec<String>,
}

pub async fn candidates(
    client: &HttpClient,
    engine: Engine,
    key: &str,
    queries: &[String],
) -> anyhow::Result<Searched> {
    let mut urls = Vec::new();
    let mut cost_usd: Option<f64> = None;
    let mut errors = Vec::new();
    for query in queries {
        match engine.search(client, key, query).await {
            Ok((found, cost)) => {
                urls.extend(found);
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
    let candidates = Candidates::collect(
        engine.name().to_string(),
        urls.iter().map(|u| Board::from_url(u)),
    );
    Ok(Searched {
        candidates,
        cost_usd,
        errors,
    })
}
