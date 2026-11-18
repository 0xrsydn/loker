//! loker as a library: [`App`] is the one handle every adapter holds.
//!
//! Each operation is one method taking the contract's params type and returning
//! its result type (see `loker_core::api`). Adapters only translate:
//! CLI flags → params → `App` → text or JSON; MCP arguments → params → `App` →
//! structured content. A new adapter (REST, a test harness) is the same shape.

pub mod mcp;

use std::path::Path;

use loker_core::api::{self, IngestParams, SearchParams, SearchResult, Stats};
use loker_core::home::db_path;
use loker_core::model::IngestStats;
use loker_core::store::Store;
use loker_sources::HttpClient;

pub struct App {
    store: Store,
    http: HttpClient,
}

impl App {
    /// Open (or create) the database under a loker home directory.
    pub async fn open(home: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            store: Store::open(&db_path(home)).await?,
            http: loker_sources::http_client()?,
        })
    }

    pub async fn search_jobs(&self, p: SearchParams) -> api::Result<SearchResult> {
        api::search_jobs(&self.store, p).await
    }

    pub async fn stats(&self) -> api::Result<Stats> {
        api::stats(&self.store).await
    }

    pub async fn ingest(&self, p: IngestParams) -> api::Result<IngestStats> {
        loker_sources::ingest(&self.store, &self.http, p).await
    }
}
