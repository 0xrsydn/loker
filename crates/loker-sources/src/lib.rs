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

use loker_core::api::{Error, IngestParams, Result};
use loker_core::model::{IngestStats, NewJob, SourceKind};
use loker_core::store::{SourceMeta, Store};
use serde_json::Value;

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
pub fn http_client() -> anyhow::Result<HttpClient> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!("loker/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .build()?)
}

/// Fetch one source once and store the result. Fetch failures are recorded in
/// `ingestion_runs`; items that fail to transform are counted as skipped.
pub async fn ingest(store: &Store, client: &HttpClient, p: IngestParams) -> Result<IngestStats> {
    let source = Source::parse(&p.source)?;
    let meta = SourceMeta {
        id: source.id(),
        kind: source.kind(),
    };
    if p.country.is_some() && source != Source::Himalayas {
        return Err(Error::Invalid(format!(
            "`country` is not supported by {}",
            meta.id
        )));
    }
    let raw = match source.fetch(client, &p).await {
        Ok(raw) => raw,
        Err(e) => {
            store.record_failure(&meta.id, &format!("{e:#}")).await?;
            return Err(e.into());
        }
    };
    let (mut jobs, mut skipped) = (Vec::new(), 0);
    for item in raw {
        match source.transform(item) {
            Ok(job) => jobs.push(job),
            Err(e) => {
                skipped += 1;
                tracing::warn!(source = %meta.id, "skipping item: {e:#}");
            }
        }
    }
    Ok(store.ingest(&meta, jobs, skipped).await?)
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
