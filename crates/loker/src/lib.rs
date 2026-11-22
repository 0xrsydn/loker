pub mod mcp;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use loker_core::api::{
    self, CONTRACT_VERSION, DiscoverParams, DiscoverReport, DoctorReport, Error, GetJobsParams,
    GetJobsResult, IngestAllReport, IngestParams, RoleEnrichReport, RolesResult, SearchParams,
    SearchResult, SourcesEval, Stats, VerifyParams, VerifyReport,
};
use loker_core::board::{Board, BoardStatus};
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

    pub async fn ingest_all(&self) -> api::Result<IngestAllReport> {
        let (config, notice) = match self.config()? {
            Some(c) => (c, None),
            None => (config::starter(), Some(config::STARTER_NOTICE.to_string())),
        };
        let mut sources: Vec<IngestParams> = config.enabled().flat_map(|s| s.params()).collect();
        if config.discovery.ingest {
            sources.extend(self.discovered_boards(&config).await?);
        }
        let mut report = loker_sources::ingest_all(&self.store, &self.http, sources).await?;
        report.notice = notice;
        if let Some(jev) = config.providers.jev.as_ref().filter(|j| j.enabled) {
            report.roles = Some(match self.enrich_with(&jev.model, usize::MAX).await {
                Ok(r) => r,
                Err(e) => RoleEnrichReport {
                    contract_version: CONTRACT_VERSION,
                    model: jev.model.clone(),
                    errors: vec![e.to_string()],
                    ..Default::default()
                },
            });
        }
        Ok(report)
    }

    pub async fn enrich_roles(&self, limit: Option<usize>) -> api::Result<RoleEnrichReport> {
        let jev = self
            .config()?
            .and_then(|c| c.providers.jev)
            .filter(|j| j.enabled)
            .ok_or_else(|| {
                Error::Invalid(format!(
                    "no [providers.jev] in {} (see `loker config init`)",
                    config::path(&self.home).display()
                ))
            })?;
        self.enrich_with(&jev.model, limit.unwrap_or(usize::MAX))
            .await
    }

    async fn enrich_with(&self, model: &str, limit: usize) -> api::Result<RoleEnrichReport> {
        let key = std::env::var("OPENROUTER_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
            .ok_or_else(|| Error::Invalid("OPENROUTER_API_KEY is not set".into()))?;
        loker_sources::jev::enrich_roles(&self.store, &self.http, model, &key, limit)
            .await
            .map_err(Error::Internal)
    }

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
                .flat_map(|id| configured(&id).unwrap_or_else(|| vec![plain(id)]))
                .collect()
        } else if let Some(c) = &config {
            c.enabled().flat_map(|s| s.params()).collect()
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

    async fn discovered_boards(&self, config: &Config) -> api::Result<Vec<IngestParams>> {
        let declared: BTreeSet<String> = config
            .sources
            .iter()
            .map(|s| Board::from_id(&s.id).map_or_else(|| s.id.clone(), |b| b.key()))
            .collect();
        Ok(self
            .store
            .boards_with_status(BoardStatus::Live, None)
            .await?
            .into_iter()
            .filter(|b| !declared.contains(&b.key()))
            .map(|b| IngestParams {
                source: b.to_string(),
                ..Default::default()
            })
            .collect())
    }

    pub async fn discover(&self, p: DiscoverParams) -> api::Result<DiscoverReport> {
        loker_sources::discover::discover(&self.store, &self.http, p).await
    }

    pub fn config_check(&self) -> api::Result<Option<Vec<config::SourceStatus>>> {
        Ok(self.config()?.map(|c| c.check(&known_source)))
    }

    pub async fn doctor(&self, live: bool) -> api::Result<DoctorReport> {
        let config = config::load(&self.home).map_err(|e| format!("{e:#}"));
        let known = |id: &str| {
            loker_sources::Source::parse(id)
                .map(|_| ())
                .map_err(|e| e.to_string())
        };
        let report = api::doctor(&self.store, &self.home, config.clone(), &known).await?;
        if !live {
            return Ok(report);
        }
        let config = config.ok().flatten().unwrap_or_else(config::starter);
        let targets = config.enabled().flat_map(|s| s.params()).collect();
        let verified = loker_sources::verify(&self.http, targets).await?;
        Ok(report.extend(api::live_checks(&verified)))
    }

    pub fn config(&self) -> api::Result<Option<Config>> {
        config::load(&self.home).map_err(|e| Error::Invalid(format!("{e:#}")))
    }

    pub fn home(&self) -> &Path {
        &self.home
    }
}

fn known_source(id: &str) -> Result<(), String> {
    loker_sources::Source::parse(id)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

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
        ("role_enrich_report.json", schema!(RoleEnrichReport)),
        ("stats.json", schema!(Stats)),
        ("sources_eval.json", schema!(SourcesEval)),
        ("ingest_params.json", schema!(IngestParams)),
        ("ingest_stats.json", schema!(IngestStats)),
        ("ingest_all_report.json", schema!(IngestAllReport)),
        ("verify_params.json", schema!(VerifyParams)),
        ("verify_report.json", schema!(VerifyReport)),
        ("doctor_report.json", schema!(DoctorReport)),
        ("discover_params.json", schema!(DiscoverParams)),
        ("discover_report.json", schema!(DiscoverReport)),
    ]
}
