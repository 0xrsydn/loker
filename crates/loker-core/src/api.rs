//! The v1 contract: one typed params struct and one typed result per operation.
//!
//! Every adapter (CLI, MCP, later REST) builds these params and returns these
//! results unchanged, so behaviour and output shape cannot drift between them.
//! The schemas come from the same derives, so MCP tool schemas and the future
//! `schema/v1/*.json` files are generated, never written by hand.
use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::country;
use crate::model::Job;
use crate::store::Store;
use crate::verdict::{Verdict, VerdictKind, Viewer, eligible_for};

/// Bumped only on breaking changes; additive changes keep it.
pub const CONTRACT_VERSION: u32 = 1;

pub const DEFAULT_LIMIT: u32 = 20;
pub const MAX_LIMIT: u32 = 100;

/// Errors every adapter maps to its own channel (exit code, MCP error, HTTP status).
#[derive(Debug)]
pub enum Error {
    /// The caller sent something unusable; safe to show as-is.
    Invalid(String),
    /// Something broke on our side.
    Internal(anyhow::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Invalid(m) => write!(f, "invalid request: {m}"),
            Error::Internal(e) => write!(f, "{e:#}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Self {
        Error::Internal(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

// ---------- search_jobs ----------

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchParams {
    /// Full-text query; every word must match. Empty lists the newest jobs.
    #[serde(default)]
    pub q: String,
    /// Where the viewer lives: ISO 3166-1 alpha-2 code or English name ("ID", "Indonesia").
    /// Adds an eligibility verdict to each job and drops jobs whose verdict is `no`.
    pub country: Option<String>,
    /// Viewer's UTC offset in hours (7, -5, 5.5). Used with `country`.
    pub tz: Option<f64>,
    /// Maximum results, default 20, at most 100.
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub contract_version: u32,
    pub count: usize,
    pub jobs: Vec<SearchHit>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    #[serde(flatten)]
    pub job: Job,
    /// Verdict for the viewer in `country`/`tz`; absent when no country was given.
    pub eligible: Option<Verdict>,
}

pub async fn search_jobs(store: &Store, p: SearchParams) -> Result<SearchResult> {
    let limit = p.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let viewer = p
        .country
        .as_deref()
        .map(|c| {
            country::to_alpha2(c)
                .map(|code| Viewer {
                    country: code.to_string(),
                    tz: p.tz,
                    accept_onsite_in: Vec::new(),
                })
                .ok_or_else(|| Error::Invalid(format!("unknown country `{c}`")))
        })
        .transpose()?;
    if let Some(tz) = p.tz
        && !(-12.0..=14.0).contains(&tz)
    {
        return Err(Error::Invalid(format!("tz {tz} is outside -12..=14")));
    }

    // Fetch wider than `limit` so eligibility filtering still fills the page.
    let candidates = store.search(&p.q, (limit * 10).max(200)).await?;
    let mut jobs = Vec::new();
    for job in candidates {
        let eligible = viewer.as_ref().map(|v| eligible_for(&job.eligibility, v));
        if eligible.as_ref().is_some_and(|v| v.kind == VerdictKind::No) {
            continue;
        }
        jobs.push(SearchHit { job, eligible });
        if jobs.len() as u32 >= limit {
            break;
        }
    }
    Ok(SearchResult {
        contract_version: CONTRACT_VERSION,
        count: jobs.len(),
        jobs,
    })
}

// ---------- stats ----------

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub contract_version: u32,
    /// Active jobs.
    pub total: i64,
    /// Active jobs per eligibility scope.
    pub by_scope: Vec<ScopeCount>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ScopeCount {
    pub scope: String,
    pub count: i64,
}

pub async fn stats(store: &Store) -> Result<Stats> {
    let by_scope: Vec<ScopeCount> = store
        .counts()
        .await?
        .into_iter()
        .map(|(scope, count)| ScopeCount { scope, count })
        .collect();
    Ok(Stats {
        contract_version: CONTRACT_VERSION,
        total: by_scope.iter().map(|s| s.count).sum(),
        by_scope,
    })
}

// ---------- ingest ----------
// The params live here so the contract is in one place; the operation itself
// lives in `loker-sources`, which owns the adapters.

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IngestParams {
    /// Source id, e.g. `himalayas`.
    pub source: String,
    /// Pages to fetch (source-defined size), default 3.
    pub pages: Option<u32>,
    /// Ask the source for jobs open to this country only (ISO alpha-2), if it supports that.
    pub country: Option<String>,
}
