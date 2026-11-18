//! Source adapters. Each adapter is two functions: `fetch` (HTTP → raw JSON
//! items) and `transform` (raw item → normalized `NewJob`). [`Source`] names a
//! source instance and dispatches to its adapter; [`ingest`] is the one entry
//! point every caller uses, so fetch, transform and store always run the same way.
//!
//! Attribution: Remote OK and Jobicy require crediting them with a link to the
//! listing. Every job keeps its per-source URLs (`Job::sources`) for that.

pub mod ashby;
pub mod greenhouse;
pub mod himalayas;
pub mod jobicy;
pub mod remoteok;

use loker_core::api::{
    CONTRACT_VERSION, Check, CheckStatus, Error, IngestParams, Result, SourceCheck, VerifyParams,
    VerifyReport,
};
use loker_core::model::{IngestStats, NewJob, SourceKind};
use loker_core::store::{Batch, SourceMeta, Store};
use serde_json::Value;
use std::collections::BTreeSet;

pub use reqwest::Client as HttpClient;

/// Source instances this build understands; `<board>` is the company's slug on that ATS.
pub const SOURCES: &[&str] = &[
    "himalayas",
    "jobicy",
    "remoteok",
    "greenhouse:<board>",
    "ashby:<board>",
];

/// A source instance: an aggregator, or one company's board on an ATS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Himalayas,
    Jobicy,
    RemoteOk,
    Greenhouse(String),
    Ashby(String),
}

impl Source {
    pub fn parse(id: &str) -> Result<Self> {
        let bad = || {
            Error::Invalid(format!(
                "unknown source `{id}` (available: {})",
                SOURCES.join(", ")
            ))
        };
        let board = |b: &str| {
            let ok = !b.is_empty()
                && b.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            ok.then(|| b.to_string()).ok_or_else(bad)
        };
        match id.split_once(':') {
            None => match id {
                "himalayas" => Ok(Source::Himalayas),
                "jobicy" => Ok(Source::Jobicy),
                "remoteok" => Ok(Source::RemoteOk),
                _ => Err(bad()),
            },
            Some(("greenhouse", b)) => Ok(Source::Greenhouse(board(b)?)),
            Some(("ashby", b)) => Ok(Source::Ashby(board(b)?)),
            Some(_) => Err(bad()),
        }
    }

    /// Stable id stored with every job, e.g. `greenhouse:binance`.
    pub fn id(&self) -> String {
        match self {
            Source::Himalayas => "himalayas".into(),
            Source::Jobicy => "jobicy".into(),
            Source::RemoteOk => "remoteok".into(),
            Source::Greenhouse(b) => format!("greenhouse:{}", b.to_lowercase()),
            Source::Ashby(b) => format!("ashby:{}", b.to_lowercase()),
        }
    }

    pub fn kind(&self) -> SourceKind {
        match self {
            Source::Greenhouse(_) | Source::Ashby(_) => SourceKind::Employer,
            _ => SourceKind::Aggregator,
        }
    }

    /// The id of a raw item, for sources whose fetch is the complete list of
    /// open jobs (a company's whole board). `None` for partial feeds, whose
    /// missing items mean nothing.
    pub fn complete_id(&self, raw: &Value) -> Option<Option<String>> {
        match self {
            Source::Greenhouse(_) | Source::Ashby(_) => Some(raw.get("id").and_then(as_string)),
            _ => None,
        }
    }

    async fn fetch(&self, client: &HttpClient, p: &IngestParams) -> anyhow::Result<Vec<Value>> {
        let pages = p.pages.unwrap_or(3).clamp(1, 50);
        match self {
            Source::Himalayas => himalayas::fetch(client, pages, p.country.as_deref()).await,
            Source::Jobicy => jobicy::fetch(client).await,
            Source::RemoteOk => remoteok::fetch(client).await,
            Source::Greenhouse(b) => greenhouse::fetch(client, b).await,
            Source::Ashby(b) => ashby::fetch(client, b).await,
        }
    }

    fn transform(&self, raw: Value) -> anyhow::Result<NewJob> {
        let id = self.id();
        match self {
            Source::Himalayas => himalayas::transform(raw),
            Source::Jobicy => jobicy::transform(raw),
            Source::RemoteOk => remoteok::transform(raw),
            Source::Greenhouse(b) => greenhouse::transform(&id, b, raw),
            Source::Ashby(b) => ashby::transform(&id, b, raw),
        }
    }
}

/// HTTP client with loker's User-Agent: the tool and its version, no personal details.
///
/// TLS trusts the platform's roots plus Mozilla's bundled ones, so the binary
/// also works where the system has no CA certificates (minimal containers,
/// the Nix build sandbox).
pub fn http_client() -> anyhow::Result<HttpClient> {
    let bundled = webpki_root_certs::TLS_SERVER_ROOT_CERTS
        .iter()
        .filter_map(|der| reqwest::Certificate::from_der(der).ok());
    Ok(reqwest::Client::builder()
        .user_agent(concat!("loker/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .tls_certs_merge(bundled)
        .build()?)
}

/// One fetch of one source, transformed but not stored. Ingest stores it;
/// verify checks it.
pub struct Fetched {
    pub source: Source,
    /// Raw items the source returned.
    pub raw_count: u32,
    pub jobs: Vec<NewJob>,
    /// Why items failed to transform (one entry per failed item).
    pub errors: Vec<String>,
    /// Every raw item's id, when the fetch is the source's complete list and
    /// every item has an id; listings missing from it can be closed.
    pub complete_ids: Option<BTreeSet<String>>,
}

pub async fn fetch_batch(client: &HttpClient, p: &IngestParams) -> Result<Fetched> {
    let source = Source::parse(&p.source)?;
    if p.country.is_some() && source != Source::Himalayas {
        return Err(Error::Invalid(format!(
            "`country` is not supported by {}",
            source.id()
        )));
    }
    let raw = source.fetch(client, p).await?;
    // Ids come from the raw items, so an item that fails to transform is still
    // "present" and never closed by mistake. Any item without an id makes the
    // set untrustworthy, and nothing is closed.
    let complete_ids: Option<BTreeSet<String>> = raw
        .iter()
        .map(|item| source.complete_id(item))
        .collect::<Option<Vec<Option<String>>>>()
        .and_then(|ids| ids.into_iter().collect());
    let raw_count = raw.len() as u32;
    let (mut jobs, mut errors) = (Vec::new(), Vec::new());
    for item in raw {
        match source.transform(item) {
            Ok(job) => jobs.push(job),
            Err(e) => errors.push(format!("{e:#}")),
        }
    }
    Ok(Fetched {
        source,
        raw_count,
        jobs,
        errors,
        complete_ids,
    })
}

/// Fetch one source once and store the result. Fetch failures are recorded in
/// `ingestion_runs`; items that fail to transform are counted as skipped.
pub async fn ingest(store: &Store, client: &HttpClient, p: IngestParams) -> Result<IngestStats> {
    let source = Source::parse(&p.source)?;
    let f = match fetch_batch(client, &p).await {
        Ok(f) => f,
        Err(Error::Internal(e)) => {
            store
                .record_failure(&source.id(), &format!("{e:#}"))
                .await?;
            return Err(Error::Internal(e));
        }
        Err(e) => return Err(e),
    };
    let meta = SourceMeta {
        id: source.id(),
        kind: source.kind(),
    };
    for e in &f.errors {
        tracing::warn!(source = %meta.id, "skipping item: {e}");
    }
    let stats = store
        .ingest(Batch {
            source: meta.clone(),
            jobs: f.jobs,
            skipped: f.errors.len() as u32,
            complete_ids: f.complete_ids,
            now: chrono::Utc::now(),
        })
        .await?;
    if stats.held > 0 {
        tracing::warn!(
            source = %meta.id,
            held = stats.held,
            "fetch looks truncated; not closing listings it is missing"
        );
    }
    Ok(stats)
}

/// Fetch each source once (without storing anything) and check what came back
/// against invariants every adapter must keep. This is how upstream API
/// changes show up: an empty feed, items that stop parsing, missing fields.
pub async fn verify(store: &Store, client: &HttpClient, p: VerifyParams) -> Result<VerifyReport> {
    let mut ids = p.sources;
    if ids.is_empty() {
        ids = store.source_ids().await?;
    }
    if ids.is_empty() {
        return Err(Error::Invalid(
            "nothing ingested yet; name the sources to verify".into(),
        ));
    }
    let mut sources = Vec::new();
    for id in ids {
        Source::parse(&id)?;
        // One page is enough to check the shape, and keeps verify light on the source.
        let params = IngestParams {
            source: id.clone(),
            pages: Some(1),
            country: None,
        };
        let check = match fetch_batch(client, &params).await {
            Ok(f) => check_fetch(&f, chrono::Utc::now()),
            Err(e) => SourceCheck {
                source: id,
                status: CheckStatus::Fail,
                fetched: 0,
                checks: vec![Check::new("fetch", CheckStatus::Fail, e.to_string())],
            },
        };
        sources.push(check);
    }
    Ok(VerifyReport {
        contract_version: CONTRACT_VERSION,
        ok: sources.iter().all(|s| s.status != CheckStatus::Fail),
        sources,
    })
}

/// Invariants for one fetch. Fail = the adapter or the source is broken;
/// warn = worth a look.
pub fn check_fetch(f: &Fetched, now: chrono::DateTime<chrono::Utc>) -> SourceCheck {
    use CheckStatus::*;
    let n = f.raw_count;
    let mut checks = Vec::new();
    let share = |k: usize| {
        if f.jobs.is_empty() {
            0.0
        } else {
            k as f64 / f.jobs.len() as f64
        }
    };

    checks.push(if n == 0 {
        Check::new("non_empty", Fail, "the source returned no items".into())
    } else {
        Check::new("non_empty", Pass, format!("{n} items"))
    });

    let failed = f.errors.len();
    let rate = if n == 0 {
        0.0
    } else {
        failed as f64 / n as f64
    };
    let first = f
        .errors
        .first()
        .map(|e| format!(": {e}"))
        .unwrap_or_default();
    checks.push(Check::new(
        "parse",
        if rate > 0.2 {
            Fail
        } else if failed > 0 {
            Warn
        } else {
            Pass
        },
        format!("{failed} of {n} items failed to parse{first}"),
    ));

    let mut seen = BTreeSet::new();
    let dups = f
        .jobs
        .iter()
        .filter(|j| !seen.insert(&j.source_job_id))
        .count();
    checks.push(Check::new(
        "unique_ids",
        if dups > 0 { Fail } else { Pass },
        format!("{dups} duplicate ids"),
    ));

    let bad: Vec<&NewJob> = f
        .jobs
        .iter()
        .filter(|j| {
            j.title.trim().is_empty()
                || j.company.trim().is_empty()
                || !(j.canonical_url.starts_with("https://")
                    || j.canonical_url.starts_with("http://"))
        })
        .collect();
    checks.push(Check::new(
        "required_fields",
        if bad.is_empty() { Pass } else { Fail },
        match bad.first() {
            Some(j) => format!(
                "{} jobs missing title, company or URL (e.g. id {})",
                bad.len(),
                j.source_job_id
            ),
            None => "title, company and URL on every job".into(),
        },
    ));

    if f.source.complete_id(&serde_json::Value::Null).is_some() {
        checks.push(match &f.complete_ids {
            Some(ids) if ids.len() as u32 == n => Check::new(
                "complete_ids",
                Pass,
                "every item has an id; closures are safe".into(),
            ),
            _ => Check::new(
                "complete_ids",
                Fail,
                "items without ids: listings missing from this source would never close".into(),
            ),
        });
    }

    let posted = f.jobs.iter().filter(|j| j.posted_at.is_some()).count();
    let future = f
        .jobs
        .iter()
        .filter(|j| {
            j.posted_at
                .is_some_and(|t| t > now + chrono::Duration::days(1))
        })
        .count();
    checks.push(Check::new(
        "posted_at",
        if future > 0 || (n > 0 && posted == 0) {
            Warn
        } else {
            Pass
        },
        format!(
            "{:.0}% dated, {future} in the future",
            share(posted) * 100.0
        ),
    ));

    let known = f
        .jobs
        .iter()
        .filter(|j| j.eligibility.scope != loker_core::model::Scope::Unknown)
        .count();
    checks.push(Check::new(
        "eligibility",
        Pass,
        format!("{:.0}% with a known scope", share(known) * 100.0),
    ));

    let status = checks.iter().map(|c| c.status).max().unwrap_or(Pass);
    SourceCheck {
        source: f.source.id(),
        status,
        fetched: n,
        checks,
    }
}

// ---------- helpers shared by adapters ----------

async fn get_json(client: &HttpClient, url: &str) -> anyhow::Result<Value> {
    use anyhow::Context;
    Ok(client
        .get(url)
        .send()
        .await?
        .error_for_status()
        .with_context(|| format!("GET {url}"))?
        .json()
        .await?)
}

/// Decode the HTML entities job feeds actually use. `&amp;` goes last so
/// `&amp;lt;` stays `&lt;`.
fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#039;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

/// HTML → Markdown; `None` for empty input.
fn html_to_md(html: Option<&str>) -> anyhow::Result<Option<String>> {
    match html.map(str::trim).filter(|h| !h.is_empty()) {
        Some(h) => Ok(Some(htmd::convert(h)?)),
        None => Ok(None),
    }
}

/// Many feeds encode numbers and ids as either strings or numbers.
fn as_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn as_f64(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}
