//! loker as a library: [`App`] is the one handle every adapter holds.
//!
//! Each operation is one method taking the contract's params type and returning
//! its result type (see `loker_core::api`). Adapters only translate:
//! CLI flags → params → `App` → text or JSON; MCP arguments → params → `App` →
//! structured content. A new adapter (REST, a test harness) is the same shape.

pub mod mcp;

use std::path::{Path, PathBuf};

use loker_core::api::{
    self, Error, GetJobsParams, GetJobsResult, IngestAllReport, IngestParams, RolesResult,
    SearchParams, SearchResult, SourcesEval, Stats, VerifyParams, VerifyReport,
};
use loker_core::config::{self, Config};
use loker_core::home::db_path;
use loker_core::model::IngestStats;
use loker_core::store::Store;
use loker_sources::HttpClient;

pub struct App {
    home: PathBuf,
    store: Store,
    http: HttpClient,
}

impl App {
    /// Open (or create) the database under a loker home directory.
    pub async fn open(home: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            home: home.to_path_buf(),
            store: Store::open(&db_path(home)).await?,
            http: loker_sources::http_client()?,
        })
    }

    pub async fn search_jobs(&self, p: SearchParams) -> api::Result<SearchResult> {
        api::search_jobs(&self.store, p).await
    }

    pub async fn get_jobs(&self, p: GetJobsParams) -> api::Result<GetJobsResult> {
        api::get_jobs(&self.store, p).await
    }

    pub fn list_roles(&self) -> RolesResult {
        api::list_roles()
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

    /// Ingest every enabled source in config.toml.
    pub async fn ingest_all(&self) -> api::Result<IngestAllReport> {
        let config = self.config()?.ok_or_else(|| {
            Error::Invalid(format!(
                "no {} yet (`loker config init` writes a starting one)",
                config::path(&self.home).display()
            ))
        })?;
        let sources = config.enabled().map(|s| s.params()).collect();
        loker_sources::ingest_all(&self.store, &self.http, sources).await
    }

    /// Named sources use their config.toml parameters when listed there; with
    /// no names, the enabled config sources (or, without a config, every
    /// source in the store).
    pub async fn verify(&self, p: VerifyParams) -> api::Result<VerifyReport> {
        let config = self.config()?;
        let configured = |id: &str| {
            config
                .as_ref()
                .and_then(|c| c.sources.iter().find(|s| s.id == id))
                .map(|s| s.params())
        };
        let plain = |id: String| IngestParams {
            source: id,
            ..Default::default()
        };
        let targets = if !p.sources.is_empty() {
            p.sources
                .into_iter()
                .map(|id| configured(&id).unwrap_or_else(|| plain(id)))
                .collect()
        } else if let Some(c) = &config {
            c.enabled().map(|s| s.params()).collect()
        } else {
            self.store
                .source_ids()
                .await?
                .into_iter()
                .map(plain)
                .collect()
        };
        loker_sources::verify(&self.http, targets).await
    }

    /// The current config.toml, read fresh so edits apply without a restart.
    pub fn config(&self) -> api::Result<Option<Config>> {
        config::load(&self.home).map_err(|e| Error::Invalid(format!("{e:#}")))
    }

    pub fn home(&self) -> &Path {
        &self.home
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
        ("get_jobs_params.json", schema!(GetJobsParams)),
        ("get_jobs_result.json", schema!(GetJobsResult)),
        ("list_roles_result.json", schema!(RolesResult)),
        ("stats.json", schema!(Stats)),
        ("sources_eval.json", schema!(SourcesEval)),
        ("ingest_params.json", schema!(IngestParams)),
        ("ingest_stats.json", schema!(IngestStats)),
        ("ingest_all_report.json", schema!(IngestAllReport)),
        ("verify_params.json", schema!(VerifyParams)),
        ("verify_report.json", schema!(VerifyReport)),
    ]
}
