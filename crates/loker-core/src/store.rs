use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::Row;
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteConnection, SqliteJournalMode, SqlitePool, SqlitePoolOptions,
};

use crate::board::{Ats, Board, BoardCheck, BoardStatus};
use crate::classify;
use crate::dedup::{self, Linked, Member, Plan};
use crate::derived;
use crate::model::{Eligibility, IngestStats, Job, JobSource, NewJob, SourceKind};
use crate::normalize::{company_key, title_key};

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

const OUTDATED_LISTINGS: &str =
    "SELECT source_id, source_job_id, canonical_job_id, company_key, title_key, 'expired'
     FROM source_jobs INDEXED BY source_jobs_expiry
     WHERE status = 'live' AND expires_at IS NOT NULL AND julianday(expires_at) < julianday(?1)
     UNION ALL
     SELECT source_id, source_job_id, canonical_job_id, company_key, title_key, 'stale'
     FROM source_jobs INDEXED BY source_jobs_stale
     WHERE status = 'live' AND julianday(fetched_at) < julianday(?2)
       AND NOT (expires_at IS NOT NULL AND julianday(expires_at) < julianday(?1))";

const SOURCES_OF_LISTING: &str = "SELECT count(DISTINCT o.source_id) FROM source_jobs s
     JOIN source_jobs o ON o.canonical_job_id = s.canonical_job_id AND +o.status = 'live'
     WHERE s.source_id = ? AND s.source_job_id = ?";

const LISTINGS_OF_KEY: &str =
    "SELECT s.source_id, s.source_job_id, s.canonical_job_id FROM source_jobs s
     JOIN canonical_jobs c ON c.id = s.canonical_job_id
     WHERE s.company_key = ? AND s.title_key = ? AND s.job IS NOT NULL AND +s.status = 'live'
     ORDER BY c.first_seen, c.id, s.source_id, s.source_job_id";

const LISTINGS_OF_SOURCE: &str =
    "SELECT source_job_id, canonical_job_id, company_key, title_key FROM source_jobs
     WHERE source_id = ? AND status = 'live'";

macro_rules! sources_col {
    () => {
        "(SELECT json_group_array(json_object('source', s.source_id, 'url', s.source_url))
          FROM source_jobs s WHERE s.canonical_job_id = c.id
            AND (s.status = 'live' OR c.status = 'closed')) AS sources"
    };
}

macro_rules! search_filters {
    () => {
        " AND c.status = 'active'
          AND (?3 IS NULL OR (IFNULL(c.scope, '') NOT IN ('onsite', 'hybrid')
               AND (c.scope <> 'countries'
                    OR COALESCE(json_extract(c.eligibility, '$.elsewhere'), 'excluded') <> 'excluded'
                    OR EXISTS (SELECT 1 FROM json_each(c.eligibility, '$.countries') WHERE value = ?3))))
          AND (?4 IS NULL OR julianday(c.posted) >= julianday(?4))
          AND (?5 IS NULL OR c.seniority IN (SELECT value FROM json_each(?5)))
          AND (?6 IS NULL OR c.min_years IS NULL OR c.min_years <= ?6)
          AND (?7 IS NULL
               OR json_extract(c.role, '$.primary') IN (SELECT value FROM json_each(?7))
               OR EXISTS (SELECT 1 FROM json_each(c.role, '$.secondary')
                          WHERE value IN (SELECT value FROM json_each(?7)))
               OR EXISTS (SELECT 1 FROM json_each(c.role, '$.modifiers')
                          WHERE value IN (SELECT value FROM json_each(?8))))"
    };
}

macro_rules! fts_from {
    () => {
        " FROM jobs_fts JOIN canonical_jobs c ON c.fts_rowid = jobs_fts.rowid WHERE jobs_fts MATCH ?1"
    };
}

macro_rules! fts_all_words {
    () => {
        "(jobs_fts.rowid IN (SELECT rowid FROM jobs_fts WHERE jobs_fts MATCH ?2))"
    };
}

macro_rules! fts_order {
    () => {
        " ORDER BY CASE WHEN ?9 THEN 0 ELSE all_words END DESC,
                   CASE WHEN ?10 THEN c.posted END DESC,
                   bm25(jobs_fts, 1.0, 5.0, 1.0, 1.0), c.id"
    };
}

macro_rules! list_from {
    () => {
        " FROM canonical_jobs c WHERE 1"
    };
}

macro_rules! list_order {
    () => {
        " ORDER BY c.posted DESC, c.id"
    };
}

macro_rules! job_cols {
    () => {
        "c.id, c.job, c.first_seen, c.status, c.role, c.seniority, c.visa, c.experience, "
    };
}

pub const STALE_AFTER_DAYS: i64 = 30;

#[derive(Debug, Clone, Default)]
pub struct SearchFilter {
    pub q: String,
    pub any_word: bool,
    pub newest: bool,
    pub country: Option<String>,
    pub posted_after: Option<String>,
    pub seniority: Option<String>,
    pub max_years: Option<i64>,
    pub roles: Option<String>,
    pub modifiers: Option<String>,
}

pub struct Candidate {
    pub id: String,
    pub partial: bool,
    pub eligibility: Eligibility,
}

type SqliteQuery<'q> = sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments>;

impl SearchFilter {
    fn fts(&self) -> Option<(String, String)> {
        fts_query(&self.q, true).zip(fts_query(&self.q, false))
    }

    fn bind<'q>(
        &'q self,
        query: SqliteQuery<'q>,
        fts: &'q Option<(String, String)>,
    ) -> SqliteQuery<'q> {
        query
            .bind(fts.as_ref().map(|f| f.0.as_str()))
            .bind(fts.as_ref().map(|f| f.1.as_str()))
            .bind(self.country.as_deref())
            .bind(self.posted_after.as_deref())
            .bind(self.seniority.as_deref())
            .bind(self.max_years)
            .bind(self.roles.as_deref())
            .bind(self.modifiers.as_deref())
    }
}

pub struct Batch {
    pub source: SourceMeta,
    pub jobs: Vec<NewJob>,
    pub skipped: u32,
    pub complete_ids: Option<BTreeSet<String>>,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct SourceMeta {
    pub id: String,
    pub kind: SourceKind,
    pub dated_by_age: bool,
}

impl Store {
    pub async fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let opts = SqliteConnectOptions::from_str("sqlite://")?
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5))
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        let store = Self { pool };
        store.rebuild_pending().await?;
        store.classify_pending().await?;
        store.derive_pending().await?;
        Ok(store)
    }

    async fn derive_pending(&self) -> anyhow::Result<()> {
        const CHUNK: i64 = 4000;
        let stamp = *derived::STAMP;
        loop {
            let pending: Vec<(String, String)> =
                sqlx::query_as("SELECT id, job FROM canonical_jobs WHERE derived_stamp < ?1 OR derived_stamp > ?1 LIMIT ?2",
                )
                .bind(stamp)
                .bind(CHUNK)
                    .fetch_all(&self.pool)
                    .await?;
            if pending.is_empty() {
                return Ok(());
            }
            let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
            let per = pending.len().div_ceil(threads);
            let cols: Vec<DerivedCols> = std::thread::scope(|scope| {
                let handles: Vec<_> = pending
                    .chunks(per)
                    .map(|chunk| {
                        scope.spawn(move || {
                            chunk
                                .iter()
                                .map(|(_, job)| DerivedCols::of(&serde_json::from_str(job)?))
                                .collect::<anyhow::Result<Vec<_>>>()
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| h.join().expect("deriving panicked"))
                    .collect::<anyhow::Result<Vec<Vec<_>>>>()
            })?
            .into_iter()
            .flatten()
            .collect();
            let mut tx = self.pool.begin().await?;
            for ((id, _), c) in pending.iter().zip(&cols) {
                c.write(&mut tx, id).await?;
            }
            tx.commit().await?;
        }
    }

    async fn classify_pending(&self) -> anyhow::Result<()> {
        let pending: Vec<(String, String)> = sqlx::query_as(
            "SELECT id, job FROM canonical_jobs WHERE role_stamp < ?1 OR role_stamp > ?1",
        )
        .bind(classify::STAMP)
        .fetch_all(&self.pool)
        .await?;
        if pending.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        for (id, job) in pending {
            let role = role_json(&mut tx, &serde_json::from_str(&job)?).await?;
            sqlx::query("UPDATE canonical_jobs SET role = ?, role_stamp = ? WHERE id = ?")
                .bind(role)
                .bind(classify::STAMP)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Rebuild every canonical job a migration left with an empty hash (0005
    /// assembles its JSON from the old columns), keeping its `last_seen`.
    async fn rebuild_pending(&self) -> anyhow::Result<()> {
        let pending: Vec<(String, String)> =
            sqlx::query_as("SELECT id, last_seen FROM canonical_jobs WHERE content_hash = ''")
                .fetch_all(&self.pool)
                .await?;
        if pending.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        for (id, last_seen) in &pending {
            rebuild_canonical(&mut tx, id, last_seen).await?;
        }
        // A job with no usable listing JSON (pre-0002 rows) keeps the assembled
        // JSON; hash it so it isn't revisited on every open.
        let left: Vec<(String, String, String)> =
            sqlx::query_as("SELECT id, status, job FROM canonical_jobs WHERE content_hash = ''")
                .fetch_all(&mut *tx)
                .await?;
        for (id, status, job) in left {
            sqlx::query("UPDATE canonical_jobs SET content_hash = ? WHERE id = ?")
                .bind(content_hash(&[&status, &job]))
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn ingest(&self, batch: Batch) -> anyhow::Result<IngestStats> {
        let Batch {
            source,
            jobs,
            skipped,
            complete_ids,
            now,
        } = batch;
        let ts = now.to_rfc3339();
        let mut stats = IngestStats {
            fetched: jobs.len() as u32 + skipped,
            skipped,
            ..Default::default()
        };
        let mut tx = self.pool.begin().await?;

        let mut touched_keys: BTreeSet<(String, String)> = BTreeSet::new();
        let mut rebuild: BTreeSet<String> = BTreeSet::new();
        let mut changed: Vec<(String, (String, String))> = Vec::new();

        for mut job in jobs {
            job.source_id = source.id.clone();
            let key = (company_key(&job.company), title_key(&job.title));
            let existing = sqlx::query(
                "SELECT canonical_job_id, content_hash, company_key, title_key, status,
                        json_extract(job, '$.postedAt') AS posted_at
                 FROM source_jobs WHERE source_id = ? AND source_job_id = ?",
            )
            .bind(&source.id)
            .bind(&job.source_job_id)
            .fetch_optional(&mut *tx)
            .await?;
            if source.dated_by_age
                && let Some(stored) = existing
                    .as_ref()
                    .and_then(|row| row.get::<Option<String>, _>("posted_at"))
                    .and_then(|t| DateTime::parse_from_rfc3339(&t).ok())
            {
                job.posted_at = Some(stored.with_timezone(&Utc));
            }
            let json = serde_json::to_string(&job)?;
            let hash = content_hash(&[&json]);
            let expires = job.expires_at.map(|t| t.to_rfc3339());

            match existing {
                Some(row) if row.get::<String, _>("content_hash") == hash => {
                    let cid: String = row.get("canonical_job_id");
                    let was_gone = row.get::<String, _>("status") == "gone";
                    sqlx::query(
                        "UPDATE source_jobs SET fetched_at = ?, status = 'live', gone_at = NULL, gone_reason = NULL
                         WHERE source_id = ? AND source_job_id = ?",
                    )
                    .bind(&ts).bind(&source.id).bind(&job.source_job_id)
                    .execute(&mut *tx)
                    .await?;
                    // Status is only ever set by `rebuild_canonical`.
                    sqlx::query("UPDATE canonical_jobs SET last_seen = ? WHERE id = ?")
                        .bind(&ts)
                        .bind(&cid)
                        .execute(&mut *tx)
                        .await?;
                    if was_gone {
                        stats.reopened += 1;
                        rebuild.insert(cid);
                        touched_keys.insert(key);
                    } else {
                        stats.unchanged += 1;
                    }
                }
                Some(row) => {
                    let mut cid: String = row.get("canonical_job_id");
                    if row.get::<String, _>("status") == "gone" {
                        stats.reopened += 1;
                    }
                    let old_key = (
                        row.get::<String, _>("company_key"),
                        row.get::<String, _>("title_key"),
                    );
                    if old_key != key {
                        let shared: i64 = sqlx::query_scalar(
                            "SELECT count(*) FROM source_jobs WHERE canonical_job_id = ?",
                        )
                        .bind(&cid)
                        .fetch_one(&mut *tx)
                        .await?;
                        if shared > 1 {
                            rebuild.insert(cid.clone());
                            cid = new_canonical(&mut tx, &job, &ts).await?;
                        }
                        touched_keys.insert(old_key);
                    }
                    sqlx::query(
                        "UPDATE source_jobs SET canonical_job_id = ?, source_url = ?, source_kind = ?, job = ?,
                           raw_data = ?, content_hash = ?, company_key = ?, title_key = ?, fetched_at = ?,
                           expires_at = ?, status = 'live', gone_at = NULL, gone_reason = NULL
                         WHERE source_id = ? AND source_job_id = ?",
                    )
                    .bind(&cid).bind(&job.source_url).bind(source.kind.as_str()).bind(&json)
                    .bind(job.raw.to_string()).bind(&hash).bind(&key.0).bind(&key.1).bind(&ts)
                    .bind(&expires).bind(&source.id).bind(&job.source_job_id)
                    .execute(&mut *tx)
                    .await?;
                    rebuild.insert(cid);
                    touched_keys.insert(key.clone());
                    changed.push((job.source_job_id.clone(), key));
                    stats.updated += 1;
                }
                None => {
                    let cid = new_canonical(&mut tx, &job, &ts).await?;
                    sqlx::query(
                        "INSERT INTO source_jobs (source_id, source_job_id, source_url, canonical_job_id, raw_data,
                           fetched_at, source_kind, job, content_hash, company_key, title_key, first_seen, expires_at)
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    )
                    .bind(&source.id).bind(&job.source_job_id).bind(&job.source_url).bind(&cid)
                    .bind(job.raw.to_string()).bind(&ts).bind(source.kind.as_str()).bind(&json)
                    .bind(&hash).bind(&key.0).bind(&key.1).bind(&ts).bind(&expires)
                    .execute(&mut *tx)
                    .await?;
                    rebuild.insert(cid);
                    touched_keys.insert(key.clone());
                    changed.push((job.source_job_id.clone(), key));
                    stats.new += 1;
                }
            }
        }

        if let Some(ids) = &complete_ids {
            let live: Vec<(String, String, String, String)> = sqlx::query_as(LISTINGS_OF_SOURCE)
                .bind(&source.id)
                .fetch_all(&mut *tx)
                .await?;
            let absent: Vec<_> = live.iter().filter(|r| !ids.contains(&r.0)).collect();
            if ids.is_empty() || absent.len() * 2 > live.len() {
                stats.held = absent.len() as u32;
            } else {
                for (sjid, cid, ck, tk) in absent {
                    mark_gone(&mut tx, &source.id, sjid, "absent", &ts).await?;
                    rebuild.insert(cid.clone());
                    touched_keys.insert((ck.clone(), tk.clone()));
                    stats.closed += 1;
                }
            }
        }

        let stale_before = (now - chrono::Duration::days(STALE_AFTER_DAYS)).to_rfc3339();
        let expired: Vec<(String, String, String, String, String, String)> =
            sqlx::query_as(OUTDATED_LISTINGS)
                .bind(&ts)
                .bind(&stale_before)
                .fetch_all(&mut *tx)
                .await?;
        for (sid, sjid, cid, ck, tk, reason) in expired {
            mark_gone(&mut tx, &sid, &sjid, &reason, &ts).await?;
            rebuild.insert(cid);
            touched_keys.insert((ck, tk));
            stats.closed += 1;
        }

        let mut ambiguous_keys = BTreeSet::new();
        for key in &touched_keys {
            if !recluster(&mut tx, key, &ts, &mut rebuild).await? {
                ambiguous_keys.insert(key.clone());
            }
        }
        for cid in &rebuild {
            rebuild_canonical(&mut tx, cid, &ts).await?;
        }

        for (source_job_id, key) in &changed {
            let sources: i64 = sqlx::query_scalar(SOURCES_OF_LISTING)
                .bind(&source.id)
                .bind(source_job_id)
                .fetch_one(&mut *tx)
                .await?;
            if sources > 1 {
                stats.merged += 1;
            }
            if ambiguous_keys.contains(key) {
                stats.ambiguous += 1;
            }
        }

        sqlx::query(
            "INSERT INTO ingestion_runs (source_id, started_at, completed_at, status, jobs_fetched, jobs_new,
               jobs_updated, jobs_unchanged, jobs_skipped, jobs_merged, jobs_ambiguous, jobs_closed,
               jobs_reopened, jobs_held)
             VALUES (?, ?, ?, 'completed', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&source.id).bind(&ts).bind(Utc::now().to_rfc3339()).bind(stats.fetched)
        .bind(stats.new).bind(stats.updated).bind(stats.unchanged).bind(stats.skipped)
        .bind(stats.merged).bind(stats.ambiguous).bind(stats.closed).bind(stats.reopened)
        .bind(stats.held)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(stats)
    }

    pub async fn live_raw(
        &self,
        source_id: &str,
    ) -> anyhow::Result<HashMap<String, serde_json::Value>> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT source_job_id, raw_data FROM source_jobs WHERE source_id = ? AND status = 'live'",
        )
        .bind(source_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(id, raw)| Some((id, serde_json::from_str(&raw).ok()?)))
            .collect())
    }

    pub async fn record_failure(&self, source_id: &str, error: &str) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO ingestion_runs (source_id, started_at, completed_at, status, error) VALUES (?, ?, ?, 'failed', ?)")
            .bind(source_id).bind(now()).bind(now()).bind(error)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn search(
        &self,
        query: &str,
        country: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> anyhow::Result<Vec<Job>> {
        let filter = SearchFilter {
            q: query.to_string(),
            country: country.map(str::to_string),
            ..Default::default()
        };
        let ids: Vec<String> = self
            .search_page(&filter, limit, offset)
            .await?
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        Ok(self
            .get_in_order(&ids)
            .await?
            .into_iter()
            .flatten()
            .collect())
    }

    pub async fn search_count(&self, f: &SearchFilter) -> anyhow::Result<(u32, u32)> {
        let fts = f.fts();
        let sql = if fts.is_some() {
            concat!(
                "SELECT count(*), IFNULL(sum(all_words), 0) FROM (SELECT ",
                fts_all_words!(),
                " AS all_words",
                fts_from!(),
                search_filters!(),
                ")"
            )
        } else {
            concat!(
                "SELECT count(*), count(*) FROM (SELECT 1",
                list_from!(),
                search_filters!(),
                ")"
            )
        };
        let (total, full): (i64, i64) = f
            .bind(sqlx::query(sql), &fts)
            .fetch_one(&self.pool)
            .await
            .map(|r| (r.get(0), r.get(1)))?;
        Ok((total as u32, full as u32))
    }

    pub async fn search_candidates(&self, f: &SearchFilter) -> anyhow::Result<Vec<Candidate>> {
        let fts = f.fts();
        let sql = if fts.is_some() {
            concat!(
                "SELECT c.id, c.eligibility, ",
                fts_all_words!(),
                " AS all_words",
                fts_from!(),
                search_filters!(),
                fts_order!()
            )
        } else {
            concat!(
                "SELECT c.id, c.eligibility, 1 AS all_words",
                list_from!(),
                search_filters!(),
                list_order!()
            )
        };
        let mut query = f.bind(sqlx::query(sql), &fts);
        if fts.is_some() {
            query = query.bind(f.any_word).bind(f.newest);
        }
        query
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(|r| {
                Ok(Candidate {
                    id: r.get("id"),
                    partial: !r.get::<bool, _>("all_words"),
                    eligibility: serde_json::from_str(&r.get::<String, _>("eligibility"))?,
                })
            })
            .collect()
    }

    pub async fn search_page(
        &self,
        f: &SearchFilter,
        limit: u32,
        offset: u32,
    ) -> anyhow::Result<Vec<(String, bool)>> {
        let fts = f.fts();
        let sql = if fts.is_some() {
            concat!(
                "SELECT c.id, ",
                fts_all_words!(),
                " AS all_words",
                fts_from!(),
                search_filters!(),
                fts_order!(),
                " LIMIT ?11 OFFSET ?12"
            )
        } else {
            concat!(
                "SELECT c.id, 1 AS all_words",
                list_from!(),
                search_filters!(),
                list_order!(),
                " LIMIT ?11 OFFSET ?12"
            )
        };
        f.bind(sqlx::query(sql), &fts)
            .bind(f.any_word)
            .bind(f.newest)
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(|r| Ok((r.get("id"), !r.try_get::<bool, _>("all_words")?)))
            .collect()
    }

    pub async fn get_in_order(&self, ids: &[String]) -> anyhow::Result<Vec<Option<Job>>> {
        let mut found: std::collections::HashMap<String, Job> = self
            .get(ids)
            .await?
            .into_iter()
            .map(|j| (j.id.clone(), j))
            .collect();
        Ok(ids.iter().map(|id| found.remove(id)).collect())
    }

    pub async fn role_questions(
        &self,
        model: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<(String, serde_json::Value)>> {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT job FROM canonical_jobs WHERE status = 'active' ORDER BY first_seen DESC, id",
        )
        .fetch_all(&self.pool)
        .await?;
        let answered: BTreeSet<String> =
            sqlx::query_scalar("SELECT key FROM role_answers WHERE model = ?")
                .bind(model)
                .fetch_all(&self.pool)
                .await?
                .into_iter()
                .collect();
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for job in rows {
            let job: NewJob = serde_json::from_str(&job)?;
            let key = classify::answer_key(&job);
            if !answered.contains(&key) && seen.insert(key.clone()) {
                out.push((key, classify::model_state(&job)));
                if out.len() == limit {
                    break;
                }
            }
        }
        Ok(out)
    }

    pub async fn put_role_answers(
        &self,
        answers: &[(String, classify::RoleAnswer)],
    ) -> anyhow::Result<u32> {
        let mut tx = self.pool.begin().await?;
        let ts = now();
        for (key, a) in answers {
            sqlx::query(
                "INSERT OR REPLACE INTO role_answers (key, model, answer, created_at) VALUES (?, ?, ?, ?)",
            )
            .bind(key)
            .bind(&a.model)
            .bind(serde_json::to_string(a)?)
            .bind(&ts)
            .execute(&mut *tx)
            .await?;
        }
        let jobs: Vec<(String, String, Option<String>)> =
            sqlx::query_as("SELECT id, job, role FROM canonical_jobs WHERE status = 'active'")
                .fetch_all(&mut *tx)
                .await?;
        let mut changed = 0;
        for (id, job, old) in jobs {
            let role = role_json(&mut tx, &serde_json::from_str(&job)?).await?;
            if role != old {
                sqlx::query("UPDATE canonical_jobs SET role = ? WHERE id = ?")
                    .bind(&role)
                    .bind(&id)
                    .execute(&mut *tx)
                    .await?;
                changed += 1;
            }
        }
        tx.commit().await?;
        Ok(changed)
    }

    pub async fn get(&self, ids: &[String]) -> anyhow::Result<Vec<Job>> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            job_cols!(),
            sources_col!(),
            " FROM canonical_jobs c WHERE c.id IN (SELECT value FROM json_each(?1))"
        ))
        .bind(serde_json::to_string(ids)?)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(row_to_job).collect()
    }

    pub async fn source_ids(&self) -> anyhow::Result<Vec<String>> {
        Ok(
            sqlx::query_scalar("SELECT DISTINCT source_id FROM source_jobs ORDER BY source_id")
                .fetch_all(&self.pool)
                .await?,
        )
    }

    pub async fn has_active_jobs(&self) -> anyhow::Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM canonical_jobs WHERE status = 'active')",
        )
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn counts(&self) -> anyhow::Result<Vec<(String, i64)>> {
        let rows = sqlx::query(
            "SELECT remote_scope, count(*) AS n FROM canonical_jobs WHERE status = 'active' GROUP BY remote_scope ORDER BY n DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|r| (r.get::<String, _>("remote_scope"), r.get::<i64, _>("n")))
            .collect())
    }

    pub async fn source_eval(&self) -> anyhow::Result<SourceEvalRows> {
        let per_source = sqlx::query(
            "WITH live AS (SELECT * FROM source_jobs WHERE job IS NOT NULL AND status = 'live'),
                  shared AS (SELECT canonical_job_id FROM live GROUP BY canonical_job_id
                             HAVING count(DISTINCT source_id) > 1),
                  gone AS (SELECT source_id, count(*) AS n FROM source_jobs WHERE status = 'gone'
                           GROUP BY source_id)
             SELECT s.source_id,
               count(*) AS jobs,
               sum(json_extract(s.job, '$.eligibility.scope') != 'unknown') AS scope_known,
               sum(json_extract(s.job, '$.eligibility.evidence') LIKE 'unmapped%') AS unmapped,
               sum(json_extract(s.job, '$.eligibility.tz') IS NOT NULL) AS tz,
               sum(json_extract(s.job, '$.pay') IS NOT NULL) AS pay,
               sum(coalesce(json_extract(s.job, '$.pay.usdMonthMax'), json_extract(s.job, '$.pay.usdMonthMin')) IS NOT NULL) AS pay_usd,
               sum(json_extract(s.job, '$.postedAt') IS NOT NULL) AS posted,
               sum(length(coalesce(json_extract(s.job, '$.descriptionMd'), '')) > 0) AS described,
               sum(s.canonical_job_id IN (SELECT canonical_job_id FROM shared)) AS shared,
               coalesce(max(g.n), 0) AS gone
             FROM live s LEFT JOIN gone g ON g.source_id = s.source_id
             GROUP BY s.source_id ORDER BY jobs DESC",
        )
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(|r| SourceRow {
            source: r.get("source_id"),
            jobs: r.get("jobs"),
            scope_known: r.get("scope_known"),
            unmapped: r.get("unmapped"),
            tz: r.get("tz"),
            pay: r.get("pay"),
            pay_usd: r.get("pay_usd"),
            posted: r.get("posted"),
            described: r.get("described"),
            shared: r.get("shared"),
            gone: r.get("gone"),
        })
        .collect();

        let overlaps = sqlx::query(
            "SELECT a.source_id AS a, b.source_id AS b, count(DISTINCT a.canonical_job_id) AS n
             FROM source_jobs a JOIN source_jobs b
               ON a.canonical_job_id = b.canonical_job_id AND a.source_id < b.source_id
             WHERE a.status = 'live' AND b.status = 'live'
             GROUP BY 1, 2 ORDER BY n DESC",
        )
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(|r| (r.get("a"), r.get("b"), r.get("n")))
        .collect();

        let conflicts = sqlx::query(
            "SELECT c.id, c.title, c.company,
               group_concat(s.source_id || '=' || json_extract(s.job, '$.eligibility.scope'), ', ') AS scopes
             FROM canonical_jobs c JOIN source_jobs s ON s.canonical_job_id = c.id
             WHERE s.job IS NOT NULL AND s.status = 'live'
               AND json_extract(s.job, '$.eligibility.scope') != 'unknown'
             GROUP BY c.id
             HAVING count(DISTINCT json_extract(s.job, '$.eligibility.scope')) > 1
             ORDER BY c.id",
        )
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(|r| {
            (
                r.get::<String, _>("id"),
                format!("{} — {}", r.get::<String, _>("title"), r.get::<String, _>("company")),
                r.get::<String, _>("scopes"),
            )
        })
        .collect();

        let blocked_keys: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM (SELECT 1 FROM source_jobs WHERE job IS NOT NULL AND status = 'live'
               GROUP BY company_key, title_key
               HAVING count(DISTINCT source_id) > 1 AND count(*) > count(DISTINCT source_id))",
        )
        .fetch_one(&self.pool)
        .await?;

        let (canonical_jobs, closed_jobs, source_jobs): (i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM canonical_jobs WHERE status = 'active'),
                    (SELECT count(*) FROM canonical_jobs WHERE status = 'closed'),
                    (SELECT count(*) FROM source_jobs WHERE status = 'live')",
        )
        .fetch_one(&self.pool)
        .await?;

        let last_runs = sqlx::query(
            "SELECT r.* FROM ingestion_runs r
             WHERE r.id = (SELECT max(id) FROM ingestion_runs WHERE source_id = r.source_id)
             ORDER BY r.source_id",
        )
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(|r| RunRow {
            source: r.get("source_id"),
            at: r.get("started_at"),
            status: r.get("status"),
            error: r.get("error"),
            fetched: r.get("jobs_fetched"),
            new: r.get("jobs_new"),
            updated: r.get("jobs_updated"),
            unchanged: r.get("jobs_unchanged"),
            merged: r.get("jobs_merged"),
            ambiguous: r.get("jobs_ambiguous"),
            skipped: r.get("jobs_skipped"),
            closed: r.get("jobs_closed"),
            reopened: r.get("jobs_reopened"),
            held: r.get("jobs_held"),
        })
        .collect();

        Ok(SourceEvalRows {
            canonical_jobs,
            closed_jobs,
            source_jobs,
            per_source,
            overlaps,
            conflicts,
            blocked_keys,
            last_runs,
        })
    }
}

pub struct SourceEvalRows {
    pub canonical_jobs: i64,
    pub closed_jobs: i64,
    pub source_jobs: i64,
    pub per_source: Vec<SourceRow>,
    pub overlaps: Vec<(String, String, i64)>,
    pub conflicts: Vec<(String, String, String)>,
    pub blocked_keys: i64,
    pub last_runs: Vec<RunRow>,
}

pub struct SourceRow {
    pub source: String,
    pub jobs: i64,
    pub scope_known: i64,
    pub unmapped: i64,
    pub tz: i64,
    pub pay: i64,
    pub pay_usd: i64,
    pub posted: i64,
    pub described: i64,
    pub shared: i64,
    pub gone: i64,
}

pub struct RunRow {
    pub source: String,
    pub at: String,
    pub status: String,
    pub error: Option<String>,
    pub fetched: Option<i64>,
    pub new: Option<i64>,
    pub updated: Option<i64>,
    pub unchanged: Option<i64>,
    pub merged: Option<i64>,
    pub ambiguous: Option<i64>,
    pub skipped: Option<i64>,
    pub closed: Option<i64>,
    pub reopened: Option<i64>,
    pub held: Option<i64>,
}

async fn recluster(
    conn: &mut SqliteConnection,
    key: &(String, String),
    ts: &str,
    rebuild: &mut BTreeSet<String>,
) -> anyhow::Result<bool> {
    let members: Vec<Member> = sqlx::query_as::<_, (String, String, String)>(LISTINGS_OF_KEY)
        .bind(&key.0)
        .bind(&key.1)
        .fetch_all(&mut *conn)
        .await?
        .into_iter()
        .map(|(source_id, source_job_id, canonical_id)| Member {
            source_id,
            source_job_id,
            canonical_id,
        })
        .collect();
    let plan = dedup::cluster(key, &members);
    match &plan {
        Plan::Keep => {}
        Plan::Merge { into, moves } => {
            for m in moves.iter().map(|&i| &members[i]) {
                relink(conn, &m.source_id, &m.source_job_id, into).await?;
                rebuild.insert(m.canonical_id.clone());
            }
            rebuild.insert(into.clone());
        }
        Plan::Split { evict } => {
            for m in evict.iter().map(|&i| &members[i]) {
                let job = load_job(conn, &m.source_id, &m.source_job_id).await?;
                let fresh = new_canonical(conn, &job, ts).await?;
                relink(conn, &m.source_id, &m.source_job_id, &fresh).await?;
                rebuild.insert(m.canonical_id.clone());
                rebuild.insert(fresh);
            }
        }
    }
    Ok(!plan.is_ambiguous())
}

async fn mark_gone(
    conn: &mut SqliteConnection,
    sid: &str,
    sjid: &str,
    reason: &str,
    ts: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE source_jobs SET status = 'gone', gone_at = ?, gone_reason = ?
         WHERE source_id = ? AND source_job_id = ?",
    )
    .bind(ts)
    .bind(reason)
    .bind(sid)
    .bind(sjid)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

async fn relink(
    conn: &mut SqliteConnection,
    sid: &str,
    sjid: &str,
    cid: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE source_jobs SET canonical_job_id = ? WHERE source_id = ? AND source_job_id = ?",
    )
    .bind(cid)
    .bind(sid)
    .bind(sjid)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

async fn load_job(conn: &mut SqliteConnection, sid: &str, sjid: &str) -> anyhow::Result<NewJob> {
    let json: String =
        sqlx::query_scalar("SELECT job FROM source_jobs WHERE source_id = ? AND source_job_id = ?")
            .bind(sid)
            .bind(sjid)
            .fetch_one(&mut *conn)
            .await?;
    Ok(serde_json::from_str(&json)?)
}

async fn new_canonical(
    conn: &mut SqliteConnection,
    job: &NewJob,
    ts: &str,
) -> anyhow::Result<String> {
    let seed = format!("{}\n{}", job.source_id, job.source_job_id);
    let mut id = job_id(&seed);
    let mut n = 0;
    while sqlx::query_scalar::<_, i64>("SELECT count(*) FROM canonical_jobs WHERE id = ?")
        .bind(&id)
        .fetch_one(&mut *conn)
        .await?
        > 0
    {
        n += 1;
        id = job_id(&format!("{seed}\n{n}"));
    }
    sqlx::query(
        "INSERT INTO canonical_jobs (id, job, content_hash, first_seen, last_seen)
         VALUES (?, ?, '', ?, ?)",
    )
    .bind(&id)
    .bind(serde_json::to_string(job)?)
    .bind(ts)
    .bind(ts)
    .execute(&mut *conn)
    .await?;
    Ok(id)
}

async fn rebuild_canonical(conn: &mut SqliteConnection, cid: &str, ts: &str) -> anyhow::Result<()> {
    let rows: Vec<(String, String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT source_kind, source_id, source_job_id, job, status FROM source_jobs WHERE canonical_job_id = ?",
    )
    .bind(cid)
    .fetch_all(&mut *conn)
    .await?;
    if rows.is_empty() {
        drop_fts(&mut *conn, cid).await?;
        sqlx::query("DELETE FROM canonical_jobs WHERE id = ?")
            .bind(cid)
            .execute(&mut *conn)
            .await?;
        return Ok(());
    }
    let linked = rows
        .into_iter()
        .map(|(kind, source_id, source_job_id, job, status)| {
            Ok(Linked {
                kind: if kind == "employer" {
                    SourceKind::Employer
                } else {
                    SourceKind::Aggregator
                },
                source_id,
                source_job_id,
                job: job.map(|j| serde_json::from_str(&j)).transpose()?,
                live: status == "live",
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let Some(dedup::Resolved { job: m, active }) = dedup::resolve(linked) else {
        return Ok(());
    };
    let status = if active { "active" } else { "closed" };

    let job = serde_json::to_string(&m)?;
    let hash = content_hash(&[status, &job]);
    let (current, closed_at): (String, Option<String>) =
        sqlx::query_as("SELECT content_hash, closed_at FROM canonical_jobs WHERE id = ?")
            .bind(cid)
            .fetch_one(&mut *conn)
            .await?;
    if current == hash {
        return Ok(());
    }
    let closed_at = (!active).then(|| closed_at.unwrap_or_else(|| ts.to_string()));
    sqlx::query(
        "UPDATE canonical_jobs SET job = ?, content_hash = ?, last_seen = ?, status = ?, closed_at = ?,
           role = ?, role_stamp = ?
         WHERE id = ?",
    )
    .bind(&job)
    .bind(&hash)
    .bind(ts)
    .bind(status)
    .bind(&closed_at)
    .bind(role_json(&mut *conn, &m).await?)
    .bind(classify::STAMP)
    .bind(cid)
    .execute(&mut *conn)
    .await?;
    DerivedCols::of(&m)?.write(&mut *conn, cid).await?;
    let kept = drop_fts(&mut *conn, cid).await?;
    let rowid = sqlx::query(
        "INSERT INTO jobs_fts (rowid, job_id, title, company, description) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(kept)
    .bind(cid)
    .bind(&m.title)
    .bind(&m.company)
    .bind(&m.description_md)
    .execute(&mut *conn)
    .await?
    .last_insert_rowid();
    sqlx::query("UPDATE canonical_jobs SET fts_rowid = ? WHERE id = ?")
        .bind(rowid)
        .bind(cid)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

async fn drop_fts(conn: &mut SqliteConnection, cid: &str) -> anyhow::Result<Option<i64>> {
    let rowid: Option<i64> =
        sqlx::query_scalar("SELECT fts_rowid FROM canonical_jobs WHERE id = ?")
            .bind(cid)
            .fetch_optional(&mut *conn)
            .await?
            .flatten();
    if let Some(r) = rowid {
        sqlx::query("DELETE FROM jobs_fts WHERE rowid = ?")
            .bind(r)
            .execute(&mut *conn)
            .await?;
    }
    Ok(rowid)
}

async fn role_json(conn: &mut SqliteConnection, job: &NewJob) -> anyhow::Result<Option<String>> {
    let answer: Option<String> =
        sqlx::query_scalar("SELECT answer FROM role_answers WHERE key = ?")
            .bind(classify::answer_key(job))
            .fetch_optional(&mut *conn)
            .await?;
    let answer: Option<classify::RoleAnswer> =
        answer.map(|a| serde_json::from_str(&a)).transpose()?;
    Ok(classify::classify_with(job, answer.as_ref())
        .map(|r| serde_json::to_string(&r))
        .transpose()?)
}

fn plain<T: serde::Serialize>(value: &T) -> anyhow::Result<String> {
    match serde_json::to_value(value)? {
        serde_json::Value::String(s) => Ok(s),
        other => anyhow::bail!("expected a string, got {other}"),
    }
}

struct DerivedCols {
    eligibility: String,
    scope: String,
    posted: Option<String>,
    seniority: Option<String>,
    visa: Option<String>,
    experience: Option<String>,
    min_years: Option<i64>,
}

impl DerivedCols {
    fn of(job: &NewJob) -> anyhow::Result<Self> {
        let d = derived::derive(job);
        Ok(Self {
            eligibility: serde_json::to_string(&job.eligibility)?,
            scope: plain(&job.eligibility.scope())?,
            posted: job.posted_at.as_ref().map(plain).transpose()?,
            seniority: d.seniority.as_ref().map(plain).transpose()?,
            visa: d.visa.as_ref().map(serde_json::to_string).transpose()?,
            min_years: d.experience.as_ref().map(|e| i64::from(e.min_years)),
            experience: d
                .experience
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?,
        })
    }

    async fn write(&self, conn: &mut SqliteConnection, id: &str) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE canonical_jobs SET eligibility = ?, scope = ?, posted = ?, seniority = ?, visa = ?,
               experience = ?, min_years = ?, derived_stamp = ? WHERE id = ?",
        )
        .bind(&self.eligibility)
        .bind(&self.scope)
        .bind(&self.posted)
        .bind(&self.seniority)
        .bind(&self.visa)
        .bind(&self.experience)
        .bind(self.min_years)
        .bind(*derived::STAMP)
        .bind(id)
        .execute(conn)
        .await?;
        Ok(())
    }
}

fn row_to_job(r: &sqlx::sqlite::SqliteRow) -> anyhow::Result<Job> {
    let parse_ts = |s: String| -> anyhow::Result<DateTime<Utc>> {
        Ok(DateTime::parse_from_rfc3339(&s)?.with_timezone(&Utc))
    };
    let mut sources: Vec<JobSource> = serde_json::from_str(&r.get::<String, _>("sources"))?;
    sources.sort_by(|a, b| a.source.cmp(&b.source));
    let m: NewJob = serde_json::from_str(&r.get::<String, _>("job"))?;
    let column = |name: &str| r.get::<Option<String>, _>(name);
    Ok(Job {
        id: r.get("id"),
        seniority: column("seniority")
            .map(|s| serde_json::from_value(serde_json::Value::String(s)))
            .transpose()?,
        title: m.title,
        company: m.company,
        employment_type: m.employment_type,
        eligibility: m.eligibility,
        pay: m.pay,
        visa: column("visa")
            .map(|v| serde_json::from_str(&v))
            .transpose()?,
        experience: column("experience")
            .map(|e| serde_json::from_str(&e))
            .transpose()?,
        tech_tags: m.tech_tags,
        url: m.apply_url.unwrap_or(m.canonical_url),
        posted_at: m.posted_at,
        first_seen: parse_ts(r.get("first_seen"))?,
        status: r.get("status"),
        role: r
            .get::<Option<String>, _>("role")
            .map(|j| serde_json::from_str(&j))
            .transpose()?,
        sources,
    })
}

/// Quote every word so user input can never break FTS5 syntax.
fn fts_query(q: &str, any_word: bool) -> Option<String> {
    let terms: Vec<String> = q
        .split_whitespace()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect();
    let sep = if any_word { " OR " } else { " " };
    (!terms.is_empty()).then(|| terms.join(sep))
}

pub fn job_id(seed: &str) -> String {
    format!("j_{}", &blake3::hash(seed.as_bytes()).to_hex()[..10])
}

fn content_hash(parts: &[&str]) -> String {
    let mut h = blake3::Hasher::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update(&[0]);
    }
    h.finalize().to_hex().to_string()
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

impl Store {
    pub async fn add_boards(&self, finder: &str, boards: &[Board]) -> anyhow::Result<u32> {
        let at = now();
        let mut tx = self.pool.begin().await?;
        let mut new = 0;
        for b in boards {
            let key = b.key();
            let added = sqlx::query(
                "INSERT INTO boards (key, ats, slug, first_found) VALUES (?, ?, ?, ?) ON CONFLICT (key) DO NOTHING",
            )
            .bind(&key)
            .bind(b.ats.as_str())
            .bind(&b.slug)
            .bind(&at)
            .execute(&mut *tx)
            .await?;
            new += added.rows_affected() as u32;
            sqlx::query(
                "INSERT INTO board_finds (key, finder, found_at) VALUES (?, ?, ?) ON CONFLICT DO NOTHING",
            )
            .bind(&key)
            .bind(finder)
            .bind(&at)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(new)
    }

    pub async fn finish_discovery(&self, finder: &str, found: u32) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO discovery_runs (finder, finished_at, found) VALUES (?, ?, ?)
             ON CONFLICT (finder) DO UPDATE SET finished_at = excluded.finished_at, found = excluded.found",
        )
        .bind(finder)
        .bind(now())
        .bind(found as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn discovered(&self, finder: &str) -> anyhow::Result<bool> {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM discovery_runs WHERE finder = ?")
            .bind(finder)
            .fetch_one(&self.pool)
            .await?;
        Ok(n > 0)
    }

    pub async fn boards_due(
        &self,
        checked_before: DateTime<Utc>,
        limit: Option<u32>,
    ) -> anyhow::Result<Vec<Board>> {
        let rows = sqlx::query(
            "SELECT ats, slug FROM boards WHERE status = 'unchecked' OR checked_at < ? \
             ORDER BY status <> 'unchecked', checked_at, first_found, key LIMIT ?",
        )
        .bind(checked_before.to_rfc3339())
        .bind(limit.map_or(-1, i64::from))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                let ats = Ats::parse(r.get::<&str, _>("ats"))?;
                Board::new(ats, r.get("slug"))
            })
            .collect())
    }

    pub async fn record_board_fetch(&self, board: &Board, jobs: Option<u32>) -> anyhow::Result<()> {
        let jobs = jobs.map(i64::from);
        sqlx::query(
            "UPDATE boards SET status = CASE WHEN ? IS NULL THEN 'missing' WHEN status = 'private' THEN 'private' \
             WHEN ? > 0 THEN 'live' ELSE 'empty' END, jobs = ?, checked_at = ?, error = NULL WHERE key = ?",
        )
        .bind(jobs)
        .bind(jobs)
        .bind(jobs)
        .bind(now())
        .bind(board.key())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn live_boards(&self) -> anyhow::Result<Vec<(Board, Option<String>)>> {
        let rows = sqlx::query(
            "SELECT ats, slug, name FROM boards WHERE status = 'live' ORDER BY first_found, key",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                let board = Board::new(Ats::parse(r.get::<&str, _>("ats"))?, r.get("slug"))?;
                Some((board, r.get("name")))
            })
            .collect())
    }

    pub async fn search_exclusions(
        &self,
        finders: &[&str],
        per_ats: u32,
    ) -> anyhow::Result<Vec<Board>> {
        let rows = sqlx::query(
            "SELECT ats, slug FROM (SELECT b.ats, b.slug, row_number() OVER (PARTITION BY b.ats \
             ORDER BY s.key IS NULL, s.seen DESC, coalesce(b.jobs, 0) DESC, b.key) AS n \
             FROM boards b LEFT JOIN \
             (SELECT key, max(found_at) AS seen FROM board_finds \
              WHERE finder IN (SELECT value FROM json_each(?)) GROUP BY key) s ON s.key = b.key \
             WHERE s.key IS NOT NULL OR b.status = 'live') WHERE n <= ? ORDER BY ats, n",
        )
        .bind(serde_json::to_string(finders)?)
        .bind(i64::from(per_ats))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .filter_map(|r| Board::new(Ats::parse(r.get::<&str, _>("ats"))?, r.get("slug")))
            .collect())
    }

    pub async fn record_board_check(
        &self,
        board: &Board,
        check: Result<&BoardCheck, &str>,
    ) -> anyhow::Result<()> {
        match check {
            Ok(c) => sqlx::query(
                "UPDATE boards SET slug = ?, status = ?, jobs = ?, name = ?, website = ?, checked_at = ?, error = NULL WHERE key = ?",
            )
            .bind(&c.slug)
            .bind(c.status.as_str())
            .bind(c.jobs.map(i64::from))
            .bind(c.company.as_ref().map(|c| c.name.as_str()))
            .bind(c.company.as_ref().and_then(|c| c.website.as_deref()))
            .bind(now())
            .bind(board.key())
            .execute(&self.pool)
            .await?,
            Err(e) => sqlx::query("UPDATE boards SET checked_at = ?, error = ? WHERE key = ?")
                .bind(now())
                .bind(e)
                .bind(board.key())
                .execute(&self.pool)
                .await?,
        };
        Ok(())
    }

    pub async fn board_counts(&self) -> anyhow::Result<Vec<(BoardStatus, bool, u32)>> {
        let rows = sqlx::query(
            "SELECT status, error IS NOT NULL AS failed, count(*) AS n FROM boards GROUP BY 1, 2",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                let status = BoardStatus::parse(r.get::<&str, _>("status"))?;
                Some((status, r.get("failed"), r.get::<i64, _>("n") as u32))
            })
            .collect())
    }
}

#[cfg(test)]
mod plans {
    use super::*;

    #[tokio::test]
    async fn ingest_lookups_use_their_index_at_any_size() {
        let dir = std::env::temp_dir().join(format!("loker-plans-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir.join("loker.db")).await.unwrap();
        for (sql, index) in [
            (OUTDATED_LISTINGS, "source_jobs_expiry"),
            (OUTDATED_LISTINGS, "source_jobs_stale"),
            (SOURCES_OF_LISTING, "source_jobs_canonical"),
            (LISTINGS_OF_KEY, "source_jobs_key"),
            (
                LISTINGS_OF_SOURCE,
                "source_jobs_live (status=? AND source_id=?)",
            ),
        ] {
            let plan: Vec<String> =
                sqlx::raw_sql(sqlx::AssertSqlSafe(format!("EXPLAIN QUERY PLAN {sql}")))
                    .fetch_all(&store.pool)
                    .await
                    .unwrap()
                    .iter()
                    .map(|r| r.get("detail"))
                    .collect();
            assert!(
                plan.iter().any(|l| l.contains(index))
                    && !plan.iter().any(|l| l.starts_with("SCAN")),
                "{sql}\n{plan:#?}"
            );
        }
    }
}
