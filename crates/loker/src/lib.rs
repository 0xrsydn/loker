//! loker as a library: [`App`] is the one handle every adapter holds.
//!
//! Each operation is one method taking the contract's params type and returning
//! its result type (see `loker_core::api`). Adapters only translate:
//! CLI flags → params → `App` → text or JSON; MCP arguments → params → `App` →
//! structured content. A new adapter (REST, a test harness) is the same shape.

pub mod mcp;

use std::path::Path;

use loker_core::api::{
    self, IngestParams, SearchParams, SearchResult, SourcesEval, Stats, VerifyParams, VerifyReport,
};
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

    pub async fn sources_eval(&self) -> api::Result<SourcesEval> {
        api::sources_eval(&self.store).await
    }

    pub async fn ingest(&self, p: IngestParams) -> api::Result<IngestStats> {
        loker_sources::ingest(&self.store, &self.http, p).await
    }

    pub async fn verify(&self, p: VerifyParams) -> api::Result<VerifyReport> {
        loker_sources::verify(&self.store, &self.http, p).await
    }
}

/// JSON Schemas of the v1 contract: every operation's params and result, by
/// file name. Committed under `schema/v1/`; the e2e test fails when they drift,
/// so contract changes are always deliberate (and additive within v1).
pub fn contract_schemas() -> Vec<(&'static str, serde_json::Value)> {
    use loker_core::model::IngestStats;
    macro_rules! schema {
        ($t:ty) => {
            serde_json::to_value(schemars::schema_for!($t)).expect("schema serializes")
        };
    }
    vec![
        ("search_params.json", schema!(SearchParams)),
        ("search_result.json", schema!(SearchResult)),
        ("stats.json", schema!(Stats)),
        ("sources_eval.json", schema!(SourcesEval)),
        ("ingest_params.json", schema!(IngestParams)),
        ("ingest_stats.json", schema!(IngestStats)),
        ("verify_params.json", schema!(VerifyParams)),
        ("verify_report.json", schema!(VerifyReport)),
    ]
}
