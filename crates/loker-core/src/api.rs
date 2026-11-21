use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use chrono::{DateTime, Utc};

use crate::country;
use crate::model::{Job, Scope, Seniority, Sponsorship};
use crate::role::{self, Modifier, Role, RoleFilter};
use crate::store::{Found, Store};
use crate::verdict::{Verdict, VerdictKind, Viewer, eligible_for};

pub const CONTRACT_VERSION: u32 = 1;

pub const DEFAULT_LIMIT: u32 = 20;
pub const MAX_LIMIT: u32 = 100;

#[derive(Debug)]
pub enum Error {
    Invalid(String),
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

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchParams {
    /// Full-text query over title, company and description; words also match
    /// their other forms ("engineer" finds "engineering"). Jobs with every word
    /// come first, then jobs with only some, marked `partial`. Keep it to the
    /// role and skills: where the viewer can work from is `country`, not a
    /// query word. Empty lists the newest jobs.
    #[serde(default)]
    pub q: String,
    /// Rank by score alone instead of putting jobs with every word first.
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
    /// Keep only jobs at these levels (`intern`, `junior`, `mid`, `senior`,
    /// `lead`, `executive`), from the title or the source. Jobs that state no
    /// level (most of them) are dropped, so leave this out to see everything.
    #[serde(default)]
    pub seniority: Vec<Seniority>,
    /// The viewer's years of experience. Drops jobs that require more
    /// (`experience.minYears` above this); jobs that state no years are kept.
    pub experience_years: Option<u8>,
    /// Keep only jobs in any of these roles or with any of these modifiers:
    /// ids from `list_roles`, at any level (`engineering` takes in `backend`
    /// and `ai_engineering`; `forward_deployed` is a modifier). Jobs with no
    /// role are dropped, so leave this out to see everything.
    #[serde(default)]
    pub role: Vec<RoleFilter>,
    /// `full` (default); `compact`: what's needed to pick jobs (id, title,
    /// company, link, scope, level, pay text, visa sponsorship, minimum years, date, verdict), about a third of
    /// the size; `titles`: id, title, company, level and verdict only, for
    /// scanning a long list. Get full details for the picks with `get_jobs`.
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
    /// Id, title, company, level and verdict: about 70 characters a job, for
    /// scanning many.
    Titles,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub contract_version: u32,
    pub count: usize,
    /// Jobs matching every filter, across all pages.
    pub total: u32,
    /// Of `total`, the jobs with every query word (the rest are `partial`).
    /// 0 means no job has all the words. Equals `total` when `q` is empty.
    pub full_matches: u32,
    /// Each job as a `SearchHit`, a `CompactHit` (`view: compact`) or a
    /// `TitleHit` (`view: titles`).
    pub jobs: Vec<Hit>,
    /// Pass as `offset` to get the next page; absent when this page is the last.
    pub next_offset: Option<u32>,
    /// Why there are no results when it isn't the query: nothing is stored
    /// yet. Absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
}

pub const EMPTY_STORE: &str = "No jobs are stored yet: run `loker ingest --all` to fetch them (`loker doctor` checks the setup).";

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    #[serde(flatten)]
    pub job: Job,
    /// Verdict for the viewer in `country`/`tz`; absent when no country was given.
    pub eligible: Option<Verdict>,
    /// Matches only some of the query words.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub partial: bool,
}

/// A job in a search result: full, or compact.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum Hit {
    Full(Box<SearchHit>),
    Compact(CompactHit),
    Titles(TitleHit),
}

/// The least needed to scan a list: open the picks with `get_jobs`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TitleHit {
    pub id: String,
    pub title: String,
    pub company: String,
    pub seniority: Option<Seniority>,
    /// Role ids, primary first; empty when nothing places the job.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<Role>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<Modifier>,
    /// The viewer's verdict (reason omitted); absent when no country was given.
    pub verdict: Option<VerdictKind>,
    /// Matches only some of the query words.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub partial: bool,
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
    /// Career level, when the title or source names one.
    pub seniority: Option<Seniority>,
    /// Role ids, primary first; empty when nothing places the job.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<Role>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<Modifier>,
    /// The source's pay text, e.g. "USD 50–100 hourly".
    pub pay: Option<String>,
    /// Visa sponsorship, when the posting states it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visa: Option<Sponsorship>,
    /// Minimum years of experience, when the posting states them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_years: Option<u8>,
    pub posted_at: Option<DateTime<Utc>>,
    /// Verdict for the viewer in `country`/`tz`; absent when no country was given.
    pub eligible: Option<Verdict>,
    /// Matches only some of the query words.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub partial: bool,
}

impl Hit {
    pub fn id(&self) -> &str {
        match self {
            Hit::Full(h) => &h.job.id,
            Hit::Compact(c) => &c.id,
            Hit::Titles(t) => &t.id,
        }
    }

    fn new(job: Job, eligible: Option<Verdict>, partial: bool, view: View) -> Self {
        let roles: Vec<Role> = job
            .role
            .iter()
            .flat_map(|r| std::iter::once(r.primary).chain(r.secondary.iter().copied()))
            .collect();
        let modifiers = job
            .role
            .as_ref()
            .map(|r| r.modifiers.clone())
            .unwrap_or_default();
        match view {
            View::Full => Hit::Full(Box::new(SearchHit {
                job,
                eligible,
                partial,
            })),
            View::Compact => Hit::Compact(CompactHit {
                scope: job.eligibility.scope(),
                seniority: job.seniority,
                roles,
                modifiers,
                pay: job.pay.map(|p| p.raw),
                visa: job.visa.map(|v| v.sponsorship),
                min_years: job.experience.as_ref().map(|e| e.min_years),
                id: job.id,
                title: job.title,
                company: job.company,
                url: job.url,
                posted_at: job.posted_at,
                eligible,
                partial,
            }),
            View::Titles => Hit::Titles(TitleHit {
                id: job.id,
                title: job.title,
                company: job.company,
                seniority: job.seniority,
                roles,
                modifiers,
                verdict: eligible.map(|v| v.kind),
                partial,
            }),
        }
    }
}

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
    let worst = match p.min_verdict {
        Some(MinVerdict::Yes) => VerdictKind::Yes,
        Some(MinVerdict::Maybe) => VerdictKind::Maybe,
        None => VerdictKind::Unknown,
    };

    const BATCH: u32 = 200;
    let offset = p.offset.unwrap_or(0);
    let country = viewer.as_ref().map(|v| v.country.as_str());
    let (mut read, mut total, mut full, mut jobs) = (0u32, 0u32, 0u32, Vec::new());
    loop {
        let rows = store
            .search_with(&p.q, p.any_word, country, BATCH, read)
            .await?;
        let n = rows.len() as u32;
        read += n;
        for Found { job, partial } in rows {
            let eligible = viewer.as_ref().map(|v| eligible_for(&job.eligibility, v));
            if eligible.as_ref().is_some_and(|v| v.kind > worst) {
                continue;
            }
            if !p.role.is_empty() && !job.role.as_ref().is_some_and(|r| r.matches(&p.role)) {
                continue;
            }
            if !p.seniority.is_empty() && !job.seniority.is_some_and(|s| p.seniority.contains(&s)) {
                continue;
            }
            if let (Some(years), Some(e)) = (p.experience_years, &job.experience)
                && e.min_years > years
            {
                continue;
            }
            total += 1;
            full += u32::from(!partial);
            if total > offset && jobs.len() < limit as usize {
                jobs.push(Hit::new(job, eligible, partial, p.view));
            }
        }
        if n < BATCH {
            break;
        }
    }
    let notice = if total == 0 && store.counts().await?.iter().all(|(_, n)| *n == 0) {
        Some(EMPTY_STORE.to_string())
    } else {
        None
    };
    Ok(SearchResult {
        contract_version: CONTRACT_VERSION,
        count: jobs.len(),
        total,
        full_matches: full,
        next_offset: (total > offset + limit).then_some(offset + limit),
        jobs,
        notice,
    })
}

/// The role vocabulary: what `role` filters and `roles` fields can name.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RolesResult {
    pub contract_version: u32,
    /// Version of the vocabulary; ids are never renamed or reused, and new
    /// ones may appear.
    pub vocabulary_version: u32,
    /// Families first, then the roles in them.
    pub roles: Vec<RoleEntry>,
    pub modifiers: Vec<ModifierEntry>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoleEntry {
    pub id: Role,
    pub label: String,
    pub definition: String,
    /// True for a family: filtering on it takes in every role inside it.
    pub is_family: bool,
    /// The family it belongs to; absent for a top-level family.
    pub family: Option<Role>,
    /// Other families whose filters also take it in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also_in: Vec<Role>,
    /// Set when the id is retired: what to use instead.
    pub replaced_by: Option<Role>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModifierEntry {
    pub id: Modifier,
    pub definition: String,
}

pub fn list_roles() -> RolesResult {
    RolesResult {
        contract_version: CONTRACT_VERSION,
        vocabulary_version: role::VERSION,
        roles: Role::ALL
            .iter()
            .map(|&r| {
                let i = r.info();
                RoleEntry {
                    id: r,
                    label: i.label.into(),
                    definition: i.definition.into(),
                    is_family: r.is_family(),
                    family: i.home,
                    also_in: i.also_in.to_vec(),
                    replaced_by: i.replaced_by,
                }
            })
            .collect(),
        modifiers: Modifier::ALL
            .iter()
            .map(|&m| ModifierEntry {
                id: m,
                definition: m.definition().into(),
            })
            .collect(),
    }
}

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
                jobs.push(SearchHit {
                    job,
                    eligible,
                    partial: false,
                });
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

/// `loker doctor`: is this install set up and healthy? Offline checks of the
/// home, config, database and each source's last fetch; with `live`, also
/// `verify`. Each problem says how to fix it.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DoctorReport {
    pub contract_version: u32,
    /// No check failed (warnings allowed).
    pub ok: bool,
    pub checks: Vec<DoctorCheck>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DoctorCheck {
    /// What was checked: "home", "config", "database", "source <id>", "provider jev", ...
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
    /// The command or edit that fixes it, when the status isn't pass.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

impl DoctorCheck {
    fn new(
        name: impl Into<String>,
        status: CheckStatus,
        detail: String,
        fix: Option<String>,
    ) -> Self {
        DoctorCheck {
            name: name.into(),
            status,
            detail,
            fix: (status != CheckStatus::Pass).then_some(fix).flatten(),
        }
    }
}

impl DoctorReport {
    pub fn new(checks: Vec<DoctorCheck>) -> Self {
        DoctorReport {
            contract_version: CONTRACT_VERSION,
            ok: checks.iter().all(|c| c.status != CheckStatus::Fail),
            checks,
        }
    }

    pub fn extend(self, more: Vec<DoctorCheck>) -> Self {
        Self::new(self.checks.into_iter().chain(more).collect())
    }
}

pub async fn doctor(
    store: &Store,
    home: &std::path::Path,
    config: std::result::Result<Option<crate::config::Config>, String>,
    known_source: &dyn Fn(&str) -> std::result::Result<(), String>,
) -> Result<DoctorReport> {
    use CheckStatus::{Fail, Pass, Warn};
    let mut checks = vec![DoctorCheck::new(
        "home",
        Pass,
        home.display().to_string(),
        None,
    )];
    let config = match config {
        Ok(Some(c)) => {
            let bad: Vec<String> = c
                .sources
                .iter()
                .filter_map(|s| known_source(&s.id).err())
                .collect();
            let n = c.enabled().count();
            checks.push(if !bad.is_empty() {
                DoctorCheck::new(
                    "config",
                    Fail,
                    bad.join("; "),
                    Some("fix the source ids in config.toml".into()),
                )
            } else if n == 0 {
                DoctorCheck::new(
                    "config",
                    Warn,
                    "no enabled source".into(),
                    Some("enable a source in config.toml".into()),
                )
            } else {
                DoctorCheck::new("config", Pass, format!("{n} enabled source(s)"), None)
            });
            c
        }
        Ok(None) => {
            checks.push(DoctorCheck::new(
                "config",
                Warn,
                "no config.toml: `loker ingest --all` uses the starter sources".into(),
                Some("loker config init --country <ISO code>".into()),
            ));
            crate::config::starter()
        }
        Err(e) => {
            checks.push(DoctorCheck::new(
                "config",
                Fail,
                e,
                Some("fix config.toml (`loker config check`)".into()),
            ));
            crate::config::starter()
        }
    };

    let total = stats(store).await?.total;
    let size = std::fs::metadata(crate::home::db_path(home))
        .map(|m| m.len())
        .unwrap_or(0);
    checks.push(if total == 0 {
        DoctorCheck::new(
            "database",
            Warn,
            "no jobs stored yet".into(),
            Some("loker ingest --all".into()),
        )
    } else {
        DoctorCheck::new(
            "database",
            Pass,
            format!("{total} active jobs, {:.0} MB", size as f64 / 1e6),
            None,
        )
    });

    let eval = sources_eval(store).await?;
    let now = Utc::now();
    let enabled: Vec<&str> = config.enabled().map(|s| s.id.as_str()).collect();
    for id in &enabled {
        let name = format!("source {id}");
        let found = eval.sources.iter().find(|s| s.source == *id);
        checks.push(match found.and_then(|s| s.last_run.as_ref()) {
            None => DoctorCheck::new(
                name,
                Warn,
                "never fetched".into(),
                Some(format!("loker ingest {id}")),
            ),
            Some(r) if r.status != "completed" => DoctorCheck::new(
                name,
                Fail,
                format!(
                    "last fetch failed ({}): {}",
                    r.at,
                    r.error.as_deref().unwrap_or("no error recorded")
                ),
                Some(format!("loker verify {id}")),
            ),
            Some(r) => {
                let hours = DateTime::parse_from_rfc3339(&r.at)
                    .map(|t| (now - t.to_utc()).num_minutes() as f64 / 60.0)
                    .unwrap_or(f64::INFINITY);
                let jobs = found.map_or(0, |s| s.jobs);
                if hours > STALE_HOURS {
                    DoctorCheck::new(
                        name,
                        Warn,
                        format!("last fetched {hours:.0} h ago; {jobs} live listings"),
                        Some("loker ingest --all, or the timer in contrib/systemd".into()),
                    )
                } else {
                    DoctorCheck::new(
                        name,
                        Pass,
                        format!("fetched {hours:.1} h ago; {jobs} live listings"),
                        None,
                    )
                }
            }
        });
    }
    for s in eval
        .sources
        .iter()
        .filter(|s| s.jobs > 0 && !enabled.contains(&s.source.as_str()))
    {
        checks.push(DoctorCheck::new(
            format!("source {}", s.source),
            Warn,
            format!(
                "{} live listings from a source not enabled; they close after 30 days unseen",
                s.jobs
            ),
            Some("enable it in config.toml, or let them close".into()),
        ));
    }

    if config.providers.jev.as_ref().is_some_and(|j| j.enabled) {
        checks.push(
            if std::env::var("OPENROUTER_API_KEY").is_ok_and(|k| !k.is_empty()) {
                DoctorCheck::new(
                    "provider jev",
                    Pass,
                    "OPENROUTER_API_KEY is set".into(),
                    None,
                )
            } else {
                DoctorCheck::new(
                    "provider jev",
                    Warn,
                    "[providers.jev] is on but OPENROUTER_API_KEY isn't set: roles stay rules-only"
                        .into(),
                    Some("set OPENROUTER_API_KEY in the environment loker runs in".into()),
                )
            },
        );
    }
    Ok(DoctorReport::new(checks))
}

const STALE_HOURS: f64 = 24.0;

pub fn live_checks(r: &VerifyReport) -> Vec<DoctorCheck> {
    r.sources
        .iter()
        .map(|s| {
            let failed: Vec<String> = s
                .checks
                .iter()
                .filter(|c| c.status != CheckStatus::Pass)
                .map(|c| format!("{}: {}", c.name, c.detail))
                .collect();
            let detail = if failed.is_empty() {
                format!("live fetch ok, {} items", s.fetched)
            } else {
                failed.join("; ")
            };
            DoctorCheck::new(
                format!("live {}", s.source),
                s.status,
                detail,
                Some(format!("loker verify {}", s.source)),
            )
        })
        .collect()
}

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

/// One `loker ingest --all` run over the configured sources.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IngestAllReport {
    pub contract_version: u32,
    /// Every source ingested without error.
    pub ok: bool,
    pub sources: Vec<IngestOutcome>,
    /// Role classification by a model, when `[providers.jev]` is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<RoleEnrichReport>,
    /// Set when there was no config.toml and the starter sources were used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
}

/// One run of role classification by a model (`loker roles enrich`).
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoleEnrichReport {
    pub contract_version: u32,
    pub model: String,
    /// Jobs with no cached answer that were asked.
    pub asked: u32,
    pub answered: u32,
    pub failed: u32,
    /// The first few failures.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    /// What the provider reported charging, in USD.
    pub cost_usd: f64,
    /// Active jobs whose role changed.
    pub reclassified: u32,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct IngestOutcome {
    pub source: String,
    pub stats: Option<crate::model::IngestStats>,
    pub error: Option<String>,
}
