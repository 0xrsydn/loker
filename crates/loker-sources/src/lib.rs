//! Source adapters. Each adapter fetches raw items and maps them to `NewJob`;
//! a shared `SourceAdapter` trait arrives with the second adapter.
//!
//! [`ingest`] is the one entry point every caller uses (CLI today, the
//! scheduler and MCP later), so fetch, transform and store run the same way.

pub mod himalayas;

use loker_core::api::{Error, IngestParams, Result};
use loker_core::model::IngestStats;
use loker_core::store::Store;

pub use reqwest::Client as HttpClient;

/// Sources this build can ingest.
pub const SOURCES: &[&str] = &[himalayas::SOURCE_ID];

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
    let pages = p.pages.unwrap_or(3).clamp(1, 50);
    let raw = match p.source.as_str() {
        himalayas::SOURCE_ID => himalayas::fetch(client, pages, p.country.as_deref()).await,
        other => {
            return Err(Error::Invalid(format!(
                "unknown source `{other}` (available: {})",
                SOURCES.join(", ")
            )));
        }
    };
    let raw = match raw {
        Ok(raw) => raw,
        Err(e) => {
            store.record_failure(&p.source, &format!("{e:#}")).await?;
            return Err(e.into());
        }
    };
    let (mut jobs, mut skipped) = (Vec::new(), 0);
    for item in raw {
        match himalayas::transform(item) {
            Ok(job) => jobs.push(job),
            Err(e) => {
                skipped += 1;
                tracing::warn!(source = %p.source, "skipping item: {e:#}");
            }
        }
    }
    Ok(store.ingest(&p.source, jobs, skipped).await?)
}
