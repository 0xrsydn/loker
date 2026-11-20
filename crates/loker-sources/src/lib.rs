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
pub mod jev;
pub mod jobicy;
pub mod lever;
pub mod remoteok;
pub mod weworkremotely;

use loker_core::api::{
    CONTRACT_VERSION, Check, CheckStatus, Error, IngestAllReport, IngestOutcome, IngestParams,
    Result, SourceCheck, VerifyReport,
};
use loker_core::model::{IngestStats, NewJob, Scope, SourceKind};
use loker_core::normalize;
use loker_core::store::{Batch, SourceMeta, Store};
use serde_json::Value;
use std::collections::BTreeSet;

pub use reqwest::Client as HttpClient;

/// Source instances this build understands; `<board>` is the company's slug on that ATS.
pub const SOURCES: &[&str] = &[
    "himalayas",
    "jobicy",
    "remoteok",
    "weworkremotely",
    "greenhouse:<board>",
    "ashby:<board>",
    "lever:<site>",
];

/// A source instance: an aggregator, or one company's board on an ATS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Himalayas,
    Jobicy,
    RemoteOk,
    WeWorkRemotely,
    Greenhouse(String),
    Ashby(String),
    Lever(String),
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
                "weworkremotely" => Ok(Source::WeWorkRemotely),
                _ => Err(bad()),
            },
            Some(("greenhouse", b)) => Ok(Source::Greenhouse(board(b)?)),
            Some(("ashby", b)) => Ok(Source::Ashby(board(b)?)),
            Some(("lever", b)) => Ok(Source::Lever(board(b)?)),
            Some(_) => Err(bad()),
        }
    }

    /// Stable id stored with every job, e.g. `greenhouse:binance`.
    pub fn id(&self) -> String {
        match self {
            Source::Himalayas => "himalayas".into(),
            Source::Jobicy => "jobicy".into(),
            Source::RemoteOk => "remoteok".into(),
            Source::WeWorkRemotely => weworkremotely::SOURCE_ID.into(),
            Source::Greenhouse(b) => format!("greenhouse:{}", b.to_lowercase()),
            Source::Ashby(b) => format!("ashby:{}", b.to_lowercase()),
            Source::Lever(b) => format!("lever:{}", b.to_lowercase()),
        }
    }

    pub fn kind(&self) -> SourceKind {
        match self {
            Source::Greenhouse(_) | Source::Ashby(_) | Source::Lever(_) => SourceKind::Employer,
            _ => SourceKind::Aggregator,
        }
    }

    /// The id of a raw item, for sources whose fetch is the complete list of
    /// open jobs (a company's whole board). `None` for partial feeds, whose
    /// missing items mean nothing.
    pub fn complete_id(&self, raw: &Value) -> Option<Option<String>> {
        match self {
            Source::Greenhouse(_) | Source::Ashby(_) | Source::Lever(_) => {
                Some(raw.get("id").and_then(as_string))
            }
            _ => None,
        }
    }

    async fn fetch(&self, client: &HttpClient, p: &IngestParams) -> anyhow::Result<Vec<Value>> {
        let pages = p.pages.unwrap_or(3).clamp(1, 50);
        match self {
            Source::Himalayas => himalayas::fetch(client, pages, p.country.as_deref()).await,
            Source::Jobicy => jobicy::fetch(client).await,
            Source::RemoteOk => remoteok::fetch(client).await,
            Source::WeWorkRemotely => weworkremotely::fetch(client).await,
            Source::Greenhouse(b) => greenhouse::fetch(client, b).await,
            Source::Ashby(b) => ashby::fetch(client, b).await,
            Source::Lever(b) => lever::fetch(client, b).await,
        }
    }

    fn transform(&self, raw: Value) -> anyhow::Result<NewJob> {
        let id = self.id();
        match self {
            Source::Himalayas => himalayas::transform(raw),
            Source::Jobicy => jobicy::transform(raw),
            Source::RemoteOk => remoteok::transform(raw),
            Source::WeWorkRemotely => weworkremotely::transform(raw),
            Source::Greenhouse(b) => greenhouse::transform(&id, b, raw),
            Source::Ashby(b) => ashby::transform(&id, b, raw),
            Source::Lever(b) => lever::transform(&id, b, raw),
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
    for (set, field) in [
        (p.company.is_some(), "company"),
        (p.remote.is_some(), "remote"),
    ] {
        if set && source.kind() != SourceKind::Employer {
            return Err(Error::Invalid(format!(
                "`{field}` only applies to employer boards, not {}",
                source.id()
            )));
        }
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
        match job_from_raw(&source, item, p.company.as_deref(), p.remote) {
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

/// One raw item as loker stores it: the adapter's transform, the board's
/// company name and remote flag (see `IngestParams`), then the cleanups every
/// job gets. Ingest uses it, and so does `examples/eligibility.rs`, which
/// re-reads stored raw items with the current rules.
pub fn job_from_raw(
    source: &Source,
    raw: Value,
    company: Option<&str>,
    remote: Option<bool>,
) -> anyhow::Result<NewJob> {
    let mut job = source.transform(raw)?;
    if let Some(name) = company {
        job.company = name.to_string();
    }
    // The board says what its postings don't: re-read their location with
    // that fact. Explicit on-site/hybrid/remote stays.
    if let Some(remote) = remote
        && job.eligibility.scope == Scope::Unknown
    {
        let raw = job.eligibility.raw.clone().unwrap_or_default();
        job.eligibility = normalize::eligibility_from_text(&raw, Some(remote));
    }
    normalize::tidy(&mut job);
    // What the board leaves open, the description may settle ("based in
    // the US"). Every aggregator here lists remote jobs only.
    let remote_board = source.kind() == SourceKind::Aggregator || remote == Some(true);
    loker_core::eligibility_text::narrow(
        &mut job.eligibility,
        job.description_md.as_deref().unwrap_or(""),
        remote_board,
    );
    Ok(job)
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
/// Each target is fetched with its own parameters (country, company), one page.
pub async fn verify(client: &HttpClient, targets: Vec<IngestParams>) -> Result<VerifyReport> {
    if targets.is_empty() {
        return Err(Error::Invalid(
            "no sources to verify: add some to config.toml or name them".into(),
        ));
    }
    let mut sources = Vec::new();
    for mut params in targets {
        Source::parse(&params.source)?;
        // One page is enough to check the shape, and keeps verify light on the source.
        params.pages = Some(1);
        let check = match fetch_batch(client, &params).await {
            Ok(f) => check_fetch(&f, chrono::Utc::now()),
            Err(e) => SourceCheck {
                source: params.source,
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

/// Ingest each source in turn. A failing source doesn't stop the others; the
/// report says which failed. Sources are fetched one at a time, a second apart.
pub async fn ingest_all(
    store: &Store,
    client: &HttpClient,
    sources: Vec<IngestParams>,
) -> Result<IngestAllReport> {
    if sources.is_empty() {
        return Err(Error::Invalid(
            "no enabled sources in config.toml (`loker config init` writes a starting one)".into(),
        ));
    }
    let mut outcomes = Vec::new();
    for (i, params) in sources.into_iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        let source = params.source.clone();
        let outcome = match ingest(store, client, params).await {
            Ok(stats) => IngestOutcome {
                source,
                stats: Some(stats),
                error: None,
            },
            Err(e) => {
                tracing::warn!(%source, "ingest failed: {e}");
                IngestOutcome {
                    source,
                    stats: None,
                    error: Some(e.to_string()),
                }
            }
        };
        outcomes.push(outcome);
    }
    Ok(IngestAllReport {
        contract_version: CONTRACT_VERSION,
        ok: outcomes.iter().all(|o| o.error.is_none()),
        sources: outcomes,
        roles: None,
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

/// `default_base` unless `LOKER_FETCH_BASE` is set, in which case every
/// source is fetched from that base instead (a local stub in the e2e test,
/// or a caching proxy).
fn url(default_base: &str, path: &str) -> String {
    match std::env::var("LOKER_FETCH_BASE") {
        Ok(base) if !base.is_empty() => format!("{}{path}", base.trim_end_matches('/')),
        _ => format!("{default_base}{path}"),
    }
}

// ---------- helpers shared by adapters ----------

async fn get_json(client: &HttpClient, url: &str) -> anyhow::Result<Value> {
    get_json_within(client, url, None).await
}

/// `get_json` with its own time limit instead of the client's 30 s, for a
/// source that is known to be slow.
async fn get_json_within(
    client: &HttpClient,
    url: &str,
    timeout: Option<std::time::Duration>,
) -> anyhow::Result<Value> {
    use anyhow::Context;
    let body = get_text_within(client, url, timeout).await?;
    serde_json::from_str(&body).with_context(|| format!("GET {url}: not JSON"))
}

/// The response body as text (RSS, HTML).
async fn get_text(client: &HttpClient, url: &str) -> anyhow::Result<String> {
    get_text_within(client, url, None).await
}

/// Transient network failures (timeout, refused or dropped connection, a body
/// cut off mid-way) are retried twice, after 2 s and 4 s. HTTP errors (a 404
/// for a missing board, a 5xx) are not: they are answers, not accidents.
async fn get_text_within(
    client: &HttpClient,
    url: &str,
    timeout: Option<std::time::Duration>,
) -> anyhow::Result<String> {
    use anyhow::Context;
    const ATTEMPTS: u32 = 3;
    let mut attempt = 1;
    loop {
        let mut req = client.get(url);
        if let Some(t) = timeout {
            req = req.timeout(t);
        }
        let result = async {
            let resp = req.send().await?.error_for_status()?;
            resp.text().await
        }
        .await;
        match result {
            Ok(v) => return Ok(v),
            Err(e) if attempt < ATTEMPTS && transient(&e) => {
                tracing::warn!(%url, attempt, "retrying after a network error: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(2u64.pow(attempt))).await;
                attempt += 1;
            }
            Err(e) => return Err(e).with_context(|| format!("GET {url}")),
        }
    }
}

/// A network failure worth another try: a timeout, a refused or dropped
/// connection, a body cut off mid-way. Never an HTTP status.
fn transient(e: &reqwest::Error) -> bool {
    !e.is_status() && (e.is_timeout() || e.is_connect() || e.is_request() || e.is_body())
}

/// A source's category parts (department, team, ...) as one string: trimmed,
/// empty and repeated parts dropped, joined with "; ". `None` when nothing is left.
fn categories<I, S>(parts: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out: Vec<String> = Vec::new();
    for p in parts {
        let p = p.as_ref().trim();
        if !p.is_empty() && !out.iter().any(|o| o == p) {
            out.push(p.to_string());
        }
    }
    (!out.is_empty()).then(|| out.join("; "))
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
