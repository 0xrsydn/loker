//! loker as a library: [`App`] is the one handle every adapter holds.
//!
//! Each operation is one method taking the contract's params type and returning
//! its result type (see `loker_core::api`). Adapters only translate:
//! CLI flags → params → `App` → text or JSON; MCP arguments → params → `App` →
//! structured content. A new adapter (REST, a test harness) is the same shape.

pub mod mcp;

use std::path::{Path, PathBuf};

use loker_core::api::{
    self, CONTRACT_VERSION, CheckStatus, DoctorCheck, DoctorReport, Error, GetJobsParams,
    GetJobsResult, IngestAllReport, IngestParams, RoleEnrichReport, RolesResult, SearchParams,
    SearchResult, SourcesEval, Stats, VerifyParams, VerifyReport,
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

    /// Ingest every enabled source in config.toml, or the starter sources
    /// when there is no config.toml yet.
    pub async fn ingest_all(&self) -> api::Result<IngestAllReport> {
        let config = self.config()?.unwrap_or_else(config::starter);
        let sources = config.enabled().map(|s| s.params()).collect();
        let mut report = loker_sources::ingest_all(&self.store, &self.http, sources).await?;
        // Roles by a model after the sources, so ingest never waits on it;
        // a failure here is reported, not fatal.
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

    /// Classify job roles with the model in `[providers.jev]`: jobs with no
    /// cached answer, at most `limit`.
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

    /// Is this install set up and healthy? Offline unless `live`, which also
    /// fetches every enabled source (as `verify`).
    pub async fn doctor(&self, live: bool) -> api::Result<DoctorReport> {
        use CheckStatus::{Fail, Pass, Warn};
        let mut checks = Vec::new();
        let mut add = |name: &str, status, detail: String, fix: Option<String>| {
            checks.push(DoctorCheck {
                name: name.to_string(),
                status,
                detail,
                fix: (status != Pass).then_some(fix).flatten(),
            })
        };
        add("home", Pass, self.home.display().to_string(), None);

        let config = match self.config() {
            Ok(Some(c)) => {
                let bad: Vec<String> = c
                    .sources
                    .iter()
                    .filter_map(|s| {
                        loker_sources::Source::parse(&s.id)
                            .err()
                            .map(|e| e.to_string())
                    })
                    .collect();
                let n = c.enabled().count();
                if !bad.is_empty() {
                    add(
                        "config",
                        Fail,
                        bad.join("; "),
                        Some("fix the source ids in config.toml".into()),
                    );
                } else if n == 0 {
                    add(
                        "config",
                        Warn,
                        "no enabled source".into(),
                        Some("enable a source in config.toml".into()),
                    );
                } else {
                    add("config", Pass, format!("{n} enabled source(s)"), None);
                }
                Some(c)
            }
            Ok(None) => {
                add(
                    "config",
                    Warn,
                    "no config.toml: `loker ingest --all` uses the starter sources".into(),
                    Some("loker config init --country <ISO code>".into()),
                );
                None
            }
            Err(e) => {
                add(
                    "config",
                    Fail,
                    e.to_string(),
                    Some("fix config.toml (`loker config check`)".into()),
                );
                None
            }
        };
        let config = config.unwrap_or_else(config::starter);

        let stats = api::stats(&self.store).await?;
        let size = std::fs::metadata(db_path(&self.home))
            .map(|m| m.len())
            .unwrap_or(0);
        if stats.total == 0 {
            add(
                "database",
                Warn,
                "no jobs stored yet".into(),
                Some("loker ingest --all".into()),
            );
        } else {
            add(
                "database",
                Pass,
                format!("{} active jobs, {:.0} MB", stats.total, size as f64 / 1e6),
                None,
            );
        }

        // Each enabled source's last fetch: never, failed, or long ago.
        let eval = api::sources_eval(&self.store).await?;
        let now = chrono::Utc::now();
        let enabled: Vec<String> = config.enabled().map(|s| s.id.clone()).collect();
        for id in &enabled {
            let name = format!("source {id}");
            let found = eval.sources.iter().find(|s| &s.source == id);
            match found.and_then(|s| s.last_run.as_ref()) {
                None => add(
                    &name,
                    Warn,
                    "never fetched".into(),
                    Some(format!("loker ingest {id}")),
                ),
                Some(r) if r.status != "completed" => add(
                    &name,
                    Fail,
                    format!(
                        "last fetch failed ({}): {}",
                        r.at,
                        r.error.as_deref().unwrap_or("no error recorded")
                    ),
                    Some(format!("loker verify {id}")),
                ),
                Some(r) => {
                    let hours = chrono::DateTime::parse_from_rfc3339(&r.at)
                        .map(|t| (now - t.to_utc()).num_minutes() as f64 / 60.0)
                        .unwrap_or(f64::INFINITY);
                    let jobs = found.map_or(0, |s| s.jobs);
                    if hours > 24.0 {
                        add(
                            &name,
                            Warn,
                            format!("last fetched {hours:.0} h ago; {jobs} live listings"),
                            Some("loker ingest --all, or the timer in contrib/systemd".into()),
                        );
                    } else {
                        add(
                            &name,
                            Pass,
                            format!("fetched {hours:.1} h ago; {jobs} live listings"),
                            None,
                        );
                    }
                }
            }
        }
        for s in eval
            .sources
            .iter()
            .filter(|s| s.jobs > 0 && !enabled.contains(&s.source))
        {
            add(
                &format!("source {}", s.source),
                Warn,
                format!(
                    "{} live listings from a source not enabled; they close after 30 days unseen",
                    s.jobs
                ),
                Some("enable it in config.toml, or let them close".into()),
            );
        }

        // Optional providers: whether the key is set, never its value.
        if config.providers.jev.as_ref().is_some_and(|j| j.enabled) {
            if std::env::var("OPENROUTER_API_KEY").is_ok_and(|k| !k.is_empty()) {
                add(
                    "provider jev",
                    Pass,
                    "OPENROUTER_API_KEY is set".into(),
                    None,
                );
            } else {
                add(
                    "provider jev",
                    Warn,
                    "[providers.jev] is on but OPENROUTER_API_KEY isn't set: roles stay rules-only"
                        .into(),
                    Some("set OPENROUTER_API_KEY in the environment loker runs in".into()),
                );
            }
        }

        if live {
            let targets = config.enabled().map(|s| s.params()).collect();
            for s in loker_sources::verify(&self.http, targets).await?.sources {
                let failed: Vec<String> = s
                    .checks
                    .iter()
                    .filter(|c| c.status != Pass)
                    .map(|c| format!("{}: {}", c.name, c.detail))
                    .collect();
                let detail = if failed.is_empty() {
                    format!("live fetch ok, {} items", s.fetched)
                } else {
                    failed.join("; ")
                };
                add(
                    &format!("live {}", s.source),
                    s.status,
                    detail,
                    Some(format!("loker verify {}", s.source)),
                );
            }
        }

        Ok(DoctorReport {
            contract_version: CONTRACT_VERSION,
            ok: checks.iter().all(|c| c.status != Fail),
            checks,
        })
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
        ("role_enrich_report.json", schema!(RoleEnrichReport)),
        ("stats.json", schema!(Stats)),
        ("sources_eval.json", schema!(SourcesEval)),
        ("ingest_params.json", schema!(IngestParams)),
        ("ingest_stats.json", schema!(IngestStats)),
        ("ingest_all_report.json", schema!(IngestAllReport)),
        ("verify_params.json", schema!(VerifyParams)),
        ("verify_report.json", schema!(VerifyReport)),
        ("doctor_report.json", schema!(DoctorReport)),
    ]
}
