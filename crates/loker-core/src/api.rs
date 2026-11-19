//! The v1 contract: one typed params struct and one typed result per operation.
//!
//! Every adapter (CLI, MCP, later REST) builds these params and returns these
//! results unchanged, so behaviour and output shape cannot drift between them.
//! The schemas come from the same derives, so MCP tool schemas and the future
//! `schema/v1/*.json` files are generated, never written by hand.
use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use chrono::{DateTime, Utc};

use crate::country;
use crate::model::{Job, Scope};
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
    /// Full-text query; every word must match (see `anyWord`). Empty lists the newest jobs.
    #[serde(default)]
    pub q: String,
    /// Match jobs with any word of `q` instead of all, best matches (most
    /// words) first. For broad queries like "api microservices golang".
    #[serde(default)]
    pub any_word: bool,
    /// Where the viewer lives: ISO 3166-1 alpha-2 code or English name ("ID", "Indonesia").
    /// Adds an eligibility verdict to each job and drops jobs whose verdict is `no`.
    pub country: Option<String>,
    /// Viewer's UTC offset in hours (7, -5, 5.5). Used with `country`.
    pub tz: Option<f64>,
    /// Maximum results, default 20, at most 100.
    pub limit: Option<u32>,
    /// Skip this many results (for the next page, pass the previous `nextOffset`).
    pub offset: Option<u32>,
    /// Keep only jobs whose verdict is at least this good: `yes`, or `maybe`
    /// (yes and maybe). Needs `country`. Without it, `unknown` jobs are kept too.
    pub min_verdict: Option<MinVerdict>,
    /// `full` (default) or `compact`: only what's needed to pick jobs (id,
    /// title, company, link, scope, pay text, date, verdict), about a third of
    /// the size. Get full details for the picks with `get_jobs`.
    #[serde(default)]
    pub view: View,
}

/// The weakest verdict a search keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MinVerdict {
    Yes,
    Maybe,
}

/// How much of each job a search returns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum View {
    #[default]
    Full,
    Compact,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub contract_version: u32,
    pub count: usize,
    /// Each job as a `SearchHit`, or a `CompactHit` with `view: compact`.
    pub jobs: Vec<Hit>,
    /// Pass as `offset` to get the next page; absent when this page is the last.
    pub next_offset: Option<u32>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    #[serde(flatten)]
    pub job: Job,
    /// Verdict for the viewer in `country`/`tz`; absent when no country was given.
    pub eligible: Option<Verdict>,
}

/// A job in a search result: full, or compact.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum Hit {
    Full(Box<SearchHit>),
    Compact(CompactHit),
}

/// Just enough of a job to choose it.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CompactHit {
    pub id: String,
    pub title: String,
    pub company: String,
    pub url: String,
    /// Eligibility scope: worldwide, regions, countries, hybrid, onsite, unknown.
    pub scope: Scope,
    /// The source's pay text, e.g. "USD 50–100 hourly".
    pub pay: Option<String>,
    pub posted_at: Option<DateTime<Utc>>,
    /// Verdict for the viewer in `country`/`tz`; absent when no country was given.
    pub eligible: Option<Verdict>,
}

impl Hit {
    pub fn id(&self) -> &str {
        match self {
            Hit::Full(h) => &h.job.id,
            Hit::Compact(c) => &c.id,
        }
    }

    fn new(job: Job, eligible: Option<Verdict>, view: View) -> Self {
        match view {
            View::Full => Hit::Full(Box::new(SearchHit { job, eligible })),
            View::Compact => Hit::Compact(CompactHit {
                scope: job.eligibility.scope,
                pay: job.pay.map(|p| p.raw),
                id: job.id,
                title: job.title,
                company: job.company,
                url: job.url,
                posted_at: job.posted_at,
                eligible,
            }),
        }
    }
}

/// The viewer described by `country` and `tz`, if a country was given.
fn viewer(country: Option<&str>, tz: Option<f64>) -> Result<Option<Viewer>> {
    if let Some(tz) = tz
        && !(-12.0..=14.0).contains(&tz)
    {
        return Err(Error::Invalid(format!("tz {tz} is outside -12..=14")));
    }
    country
        .map(|c| {
            country::to_alpha2(c)
                .map(|code| Viewer {
                    country: code.to_string(),
                    tz,
                    accept_onsite_in: Vec::new(),
                })
                .ok_or_else(|| Error::Invalid(format!("unknown country `{c}`")))
        })
        .transpose()
}

pub async fn search_jobs(store: &Store, p: SearchParams) -> Result<SearchResult> {
    let limit = p.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let viewer = viewer(p.country.as_deref(), p.tz)?;
    if p.min_verdict.is_some() && viewer.is_none() {
        return Err(Error::Invalid(
            "`minVerdict` needs `country`: a verdict is about a viewer".into(),
        ));
    }
    // Verdicts sort best first (yes, maybe, unknown, no); keep up to this one.
    let worst = match p.min_verdict {
        Some(MinVerdict::Yes) => VerdictKind::Yes,
        Some(MinVerdict::Maybe) => VerdictKind::Maybe,
        None => VerdictKind::Unknown,
    };

    // Page through the store until the page fills: the verdict can drop jobs
    // the SQL predicate keeps (time zones, on-site), so one fixed-size read
    // could come back short even when matches exist further down.
    const BATCH: u32 = 200;
    let offset = p.offset.unwrap_or(0);
    let country = viewer.as_ref().map(|v| v.country.as_str());
    let (mut to_skip, mut read, mut jobs, mut more) = (offset, 0u32, Vec::new(), false);
    'pages: loop {
        let rows = store
            .search_with(&p.q, p.any_word, country, BATCH, read)
            .await?;
        let n = rows.len() as u32;
        read += n;
        for job in rows {
            let eligible = viewer.as_ref().map(|v| eligible_for(&job.eligibility, v));
            if eligible.as_ref().is_some_and(|v| v.kind > worst) {
                continue;
            }
            if to_skip > 0 {
                to_skip -= 1;
                continue;
            }
            if jobs.len() as u32 == limit {
                more = true;
                break 'pages;
            }
            jobs.push(Hit::new(job, eligible, p.view));
        }
        if n < BATCH {
            break;
        }
    }
    Ok(SearchResult {
        contract_version: CONTRACT_VERSION,
        count: jobs.len(),
        next_offset: more.then_some(offset + limit),
        jobs,
    })
}

// ---------- get_jobs ----------

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetJobsParams {
    /// Job ids from a search, at most 100.
    pub ids: Vec<String>,
    /// Viewer's country, as in `search_jobs`, to include each job's verdict.
    pub country: Option<String>,
    /// Viewer's UTC offset in hours. Used with `country`.
    pub tz: Option<f64>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetJobsResult {
    pub contract_version: u32,
    /// The jobs found, in the order asked for, closed ones included (see `status`).
    pub jobs: Vec<SearchHit>,
    /// Ids that match no job.
    pub missing: Vec<String>,
}

/// Full details for jobs picked from a (compact) search.
pub async fn get_jobs(store: &Store, p: GetJobsParams) -> Result<GetJobsResult> {
    if p.ids.len() > MAX_LIMIT as usize {
        return Err(Error::Invalid(format!("at most {MAX_LIMIT} ids per call")));
    }
    let viewer = viewer(p.country.as_deref(), p.tz)?;
    let mut found: std::collections::HashMap<String, Job> = store
        .get(&p.ids)
        .await?
        .into_iter()
        .map(|j| (j.id.clone(), j))
        .collect();
    let (mut jobs, mut missing) = (Vec::new(), Vec::new());
    for id in p.ids {
        match found.remove(&id) {
            Some(job) => {
                let eligible = viewer.as_ref().map(|v| eligible_for(&job.eligibility, v));
                jobs.push(SearchHit { job, eligible });
            }
            None if !jobs.iter().any(|h: &SearchHit| h.job.id == id) => missing.push(id),
            None => {}
        }
    }
    Ok(GetJobsResult {
        contract_version: CONTRACT_VERSION,
        jobs,
        missing,
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

// ---------- sources_eval ----------

/// How good each source is, and how sources overlap: the numbers used to
/// decide whether a source or a normalization rule earns its keep.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourcesEval {
    pub contract_version: u32,
    /// Active canonical (deduplicated) jobs.
    pub canonical_jobs: i64,
    /// Jobs whose every listing is gone.
    pub closed_jobs: i64,
    /// Live listings (one per source per job).
    pub source_jobs: i64,
    /// Live listings per active job; 1.0 means no cross-source duplicates.
    pub listings_per_job: f64,
    pub sources: Vec<SourceEval>,
    /// Canonical jobs listed by both sources.
    pub overlaps: Vec<Overlap>,
    /// Company + title keys listed by several sources where one source has the
    /// key twice, so no merge was made. High numbers mean the key is too coarse.
    pub blocked_keys: i64,
    /// Merged jobs whose sources disagree on eligibility scope (unknown ignored).
    pub scope_conflicts: Vec<ScopeConflict>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceEval {
    pub source: String,
    /// Live listings.
    pub jobs: i64,
    /// Listings closed (absent from a full fetch, expired, or stale).
    pub gone: i64,
    /// Jobs no other source lists.
    pub unique: i64,
    /// Share of fields each normalizer filled, 0..1.
    pub coverage: Coverage,
    /// Jobs whose location text had parts no rule recognized.
    pub location_unmapped: i64,
    pub last_run: Option<RunSummary>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    /// Eligibility scope is not `unknown`.
    pub scope: f64,
    pub timezone: f64,
    /// Any pay text.
    pub pay: f64,
    /// Pay converted to USD per month.
    pub pay_usd: f64,
    pub posted_at: f64,
    pub description: f64,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub at: String,
    pub status: String,
    pub error: Option<String>,
    pub fetched: Option<i64>,
    pub new: Option<i64>,
    pub updated: Option<i64>,
    pub unchanged: Option<i64>,
    pub merged: Option<i64>,
    pub ambiguous: Option<i64>,
    pub closed: Option<i64>,
    pub reopened: Option<i64>,
    pub held: Option<i64>,
    pub skipped: Option<i64>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Overlap {
    pub a: String,
    pub b: String,
    pub shared: i64,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScopeConflict {
    pub job_id: String,
    pub job: String,
    /// "source=scope, …"
    pub scopes: String,
}

pub async fn sources_eval(store: &Store) -> Result<SourcesEval> {
    let rows = store.source_eval().await?;
    let mut runs: std::collections::BTreeMap<String, RunSummary> = rows
        .last_runs
        .into_iter()
        .map(|r| {
            (
                r.source,
                RunSummary {
                    at: r.at,
                    status: r.status,
                    error: r.error,
                    fetched: r.fetched,
                    new: r.new,
                    updated: r.updated,
                    unchanged: r.unchanged,
                    merged: r.merged,
                    ambiguous: r.ambiguous,
                    closed: r.closed,
                    reopened: r.reopened,
                    held: r.held,
                    skipped: r.skipped,
                },
            )
        })
        .collect();
    let rate = |n: i64, d: i64| {
        if d == 0 {
            0.0
        } else {
            (n as f64 / d as f64 * 1000.0).round() / 1000.0
        }
    };
    let mut sources: Vec<SourceEval> = rows
        .per_source
        .into_iter()
        .map(|s| SourceEval {
            last_run: runs.remove(&s.source),
            source: s.source,
            jobs: s.jobs,
            gone: s.gone,
            unique: s.jobs - s.shared,
            coverage: Coverage {
                scope: rate(s.scope_known, s.jobs),
                timezone: rate(s.tz, s.jobs),
                pay: rate(s.pay, s.jobs),
                pay_usd: rate(s.pay_usd, s.jobs),
                posted_at: rate(s.posted, s.jobs),
                description: rate(s.described, s.jobs),
            },
            location_unmapped: s.unmapped,
        })
        .collect();
    // Sources whose only runs failed still show up.
    sources.extend(runs.into_iter().map(|(source, run)| SourceEval {
        source,
        jobs: 0,
        gone: 0,
        unique: 0,
        coverage: Coverage {
            scope: 0.0,
            timezone: 0.0,
            pay: 0.0,
            pay_usd: 0.0,
            posted_at: 0.0,
            description: 0.0,
        },
        location_unmapped: 0,
        last_run: Some(run),
    }));
    Ok(SourcesEval {
        contract_version: CONTRACT_VERSION,
        canonical_jobs: rows.canonical_jobs,
        closed_jobs: rows.closed_jobs,
        source_jobs: rows.source_jobs,
        listings_per_job: rate(rows.source_jobs, rows.canonical_jobs),
        sources,
        overlaps: rows
            .overlaps
            .into_iter()
            .map(|(a, b, shared)| Overlap { a, b, shared })
            .collect(),
        blocked_keys: rows.blocked_keys,
        scope_conflicts: rows
            .conflicts
            .into_iter()
            .map(|(job_id, job, scopes)| ScopeConflict {
                job_id,
                job,
                scopes,
            })
            .collect(),
    })
}

// ---------- verify ----------

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerifyParams {
    /// Sources to check; default: the enabled sources in config.toml, or every
    /// source in the store when there is no config.
    #[serde(default)]
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VerifyReport {
    pub contract_version: u32,
    /// No source failed.
    pub ok: bool,
    pub sources: Vec<SourceCheck>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceCheck {
    pub source: String,
    /// The worst of its checks.
    pub status: CheckStatus,
    /// Raw items fetched.
    pub fetched: u32,
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Check {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
}

impl Check {
    pub fn new(name: &str, status: CheckStatus, detail: String) -> Self {
        Check {
            name: name.into(),
            status,
            detail,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

// ---------- ingest ----------
// The params live here so the contract is in one place; the operation itself
// lives in `loker-sources`, which owns the adapters.

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IngestParams {
    /// Source instance: `himalayas`, `jobicy`, `remoteok`, `greenhouse:<board>`, `ashby:<board>`, `lever:<site>`.
    pub source: String,
    /// Pages to fetch where the source paginates (himalayas), default 3.
    pub pages: Option<u32>,
    /// Ask the source for jobs open to this country only (ISO alpha-2), where supported (himalayas).
    pub country: Option<String>,
    /// Company display name for an employer board (`greenhouse:`/`ashby:`/`lever:`); Ashby and Lever only give the slug.
    pub company: Option<String>,
    /// What an employer board's postings don't say: `true` when every job on
    /// it is remote (a remote-first company whose locations are where you can
    /// be based), `false` when every job is on-site. Only fills in jobs whose
    /// posting doesn't state it; an explicit remote/on-site/hybrid wins.
    pub remote: Option<bool>,
}

// ---------- ingest_all ----------

/// One `loker ingest --all` run over the configured sources.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IngestAllReport {
    pub contract_version: u32,
    /// Every source ingested without error.
    pub ok: bool,
    pub sources: Vec<IngestOutcome>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct IngestOutcome {
    pub source: String,
    pub stats: Option<crate::model::IngestStats>,
    pub error: Option<String>,
}
