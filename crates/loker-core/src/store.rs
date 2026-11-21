//! SQLite store: one file, WAL mode, migrations embedded at compile time.
//!
//! Ingest is idempotent per source and dedup is order-independent:
//!
//! 1. Each source job is upserted into `source_jobs` with its normalized
//!    content, its own hash and its match key (company_key, title_key).
//! 2. Every key the batch touched is re-clustered from the stored rows alone
//!    (see [`dedup::cluster`]), so the outcome never depends on which source
//!    was ingested first.
//! 3. Every canonical job whose members changed is rebuilt from its members
//!    with a fixed precedence (see [`dedup::resolve`]).
//!
//! The rules live in [`dedup`], which does no IO; this module loads the rows
//! they need and writes their answers back.
//!
//! Liveness: a listing goes `gone` when a complete fetch of its source no
//! longer has it, when its source's expiry passes, or after [`STALE_AFTER_DAYS`]
//! unseen; it comes back `live` when seen again. Only live listings are
//! clustered, and a canonical job is `active` exactly while at least one of its
//! listings is live, so status too depends only on stored rows.
use std::collections::BTreeSet;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::Row;
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteConnection, SqliteJournalMode, SqlitePool, SqlitePoolOptions,
};

use crate::classify;
use crate::dedup::{self, Linked, Member, Plan};
use crate::model::{IngestStats, Job, JobSource, NewJob, SourceKind};
use crate::normalize::{company_key, title_key};

pub struct Store {
    pool: SqlitePool,
}

/// The `sources` column of a job row: every source listing it, for attribution.
macro_rules! sources_col {
    () => {
        "(SELECT json_group_array(json_object('source', s.source_id, 'url', s.source_url))
          FROM source_jobs s WHERE s.canonical_job_id = c.id
            AND (s.status = 'live' OR c.status = 'closed')) AS sources"
    };
}

/// Drops jobs restricted to countries that don't include the viewer (bound
/// as `?N`, NULL for no viewer). It must never be stricter than
/// `verdict::eligible_for`: search pages until the limit fills, so this is only
/// an optimization. If on-site preferences start producing `no` in the
/// verdict, they can be mirrored here too.
macro_rules! viewer_location {
    ($n:literal) => {
        concat!(
            "(?",
            $n,
            " IS NULL OR c.remote_scope <> 'countries' OR EXISTS ",
            "(SELECT 1 FROM json_each(c.job, '$.eligibility.countries') WHERE value = ?",
            $n,
            "))"
        )
    };
}

/// Listings nobody has seen for this long are closed, whatever the source.
/// A backstop for partial feeds (an aggregator's newest N) and for sources
/// that stopped being ingested.
pub const STALE_AFTER_DAYS: i64 = 30;

/// A search result: the job, and whether it matched only some query words.
pub struct Found {
    pub job: Job,
    pub partial: bool,
}

/// One fetch of one source, ready to store.
pub struct Batch {
    pub source: SourceMeta,
    pub jobs: Vec<NewJob>,
    /// Items the adapter could not transform.
    pub skipped: u32,
    /// Ids of every item in the fetch, when the fetch is the source's complete
    /// list (a company's whole board): listings missing from it are closed.
    /// `None` for partial feeds.
    pub complete_ids: Option<BTreeSet<String>>,
    /// The run's clock (tests move it).
    pub now: DateTime<Utc>,
}

/// Which source a batch came from.
#[derive(Debug, Clone)]
pub struct SourceMeta {
    /// Instance id, e.g. `himalayas` or `greenhouse:binance`.
    pub id: String,
    pub kind: SourceKind,
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
        // One connection: the CLI is a single writer. `serve` will split a read pool from a writer task.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        let store = Self { pool };
        store.rebuild_pending().await?;
        store.classify_pending().await?;
        Ok(store)
    }

    /// Classify again every job whose role was computed by other rules or
    /// another vocabulary (or never), from its stored job JSON.
    async fn classify_pending(&self) -> anyhow::Result<()> {
        let pending: Vec<(String, String)> =
            sqlx::query_as("SELECT id, job FROM canonical_jobs WHERE role_stamp != ?")
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

    /// Store one source's batch in a single transaction. Re-ingesting identical
    /// content only touches `fetched_at`/`last_seen`, so running ingest twice is a no-op.
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
        // Source jobs that are new or changed in this batch, with their key.
        let mut changed: Vec<(String, (String, String))> = Vec::new();

        for mut job in jobs {
            job.source_id = source.id.clone();
            let key = (company_key(&job.company), title_key(&job.title));
            let json = serde_json::to_string(&job)?;
            let hash = content_hash(&[&json]);
            let expires = job.expires_at.map(|t| t.to_rfc3339());
            let existing = sqlx::query(
                "SELECT canonical_job_id, content_hash, company_key, title_key, status FROM source_jobs
                 WHERE source_id = ? AND source_job_id = ?",
            )
            .bind(&source.id)
            .bind(&job.source_job_id)
            .fetch_optional(&mut *tx)
            .await?;

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
                        // The match key changed: leave the old cluster before re-clustering both keys.
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

        // Close listings a complete fetch no longer has, unless the fetch looks truncated.
        if let Some(ids) = &complete_ids {
            let live: Vec<(String, String, String, String)> = sqlx::query_as(
                "SELECT source_job_id, canonical_job_id, company_key, title_key FROM source_jobs
                 WHERE source_id = ? AND status = 'live'",
            )
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

        // Expiry and staleness, for every source.
        let stale_before = (now - chrono::Duration::days(STALE_AFTER_DAYS)).to_rfc3339();
        let expired: Vec<(String, String, String, String, String, String)> = sqlx::query_as(
            "SELECT source_id, source_job_id, canonical_job_id, company_key, title_key,
               CASE WHEN expires_at IS NOT NULL AND julianday(expires_at) < julianday(?1)
                    THEN 'expired' ELSE 'stale' END
             FROM source_jobs
             WHERE status = 'live' AND ((expires_at IS NOT NULL AND julianday(expires_at) < julianday(?1))
                                        OR julianday(fetched_at) < julianday(?2))",
        )
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
            let sources: i64 = sqlx::query_scalar(
                "SELECT count(DISTINCT o.source_id) FROM source_jobs s
                 JOIN source_jobs o ON o.canonical_job_id = s.canonical_job_id AND o.status = 'live'
                 WHERE s.source_id = ? AND s.source_job_id = ?",
            )
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

    pub async fn record_failure(&self, source_id: &str, error: &str) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO ingestion_runs (source_id, started_at, completed_at, status, error) VALUES (?, ?, ?, 'failed', ?)")
            .bind(source_id).bind(now()).bind(now()).bind(error)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Full-text search over active jobs, best match first; an empty query
    /// lists the newest. With `country`, jobs restricted to other countries
    /// are skipped in SQL (see [`viewer_location`]). `offset` counts rows of
    /// this query; ties are broken by id so pages never overlap.
    pub async fn search(
        &self,
        query: &str,
        country: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> anyhow::Result<Vec<Job>> {
        let found = self
            .search_with(query, false, country, limit, offset)
            .await?;
        Ok(found.into_iter().map(|f| f.job).collect())
    }

    /// [`Store::search`], marking which jobs match only some of the words.
    ///
    /// A job matches when it has any word of `query`. Jobs with every word
    /// come first, then the rest (`partial`), each group by score. With
    /// `any_word`, score alone orders them. eval/search measured both
    /// orders as equal; all-words-first keeps the strongest matches on top
    /// and never comes back empty when some word matches.
    pub async fn search_with(
        &self,
        query: &str,
        any_word: bool,
        country: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> anyhow::Result<Vec<Found>> {
        let (Some(any), Some(all)) = (fts_query(query, true), fts_query(query, false)) else {
            let rows = sqlx::query(concat!(
                "SELECT c.*, ",
                sources_col!(),
                " FROM canonical_jobs c WHERE c.status = 'active' AND ",
                viewer_location!("1"),
                " ORDER BY c.posted_at DESC, c.id LIMIT ?2 OFFSET ?3"
            ))
            .bind(country)
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;
            return rows
                .iter()
                .map(|r| {
                    Ok(Found {
                        job: row_to_job(r)?,
                        partial: false,
                    })
                })
                .collect();
        };
        let rows = sqlx::query(concat!(
            "SELECT c.*, ",
            sources_col!(),
            ", jobs_fts.job_id IN (SELECT job_id FROM jobs_fts WHERE jobs_fts MATCH ?5) AS all_words
             FROM jobs_fts JOIN canonical_jobs c ON c.id = jobs_fts.job_id
             WHERE jobs_fts MATCH ?1 AND c.status = 'active' AND ",
            viewer_location!("2"),
            // Column weights (job_id, title, company, description): a title
            // match counts five times. eval/search: 5 beats 1, and 10 is no
            // better.
            " ORDER BY CASE WHEN ?6 THEN 0 ELSE all_words END DESC,
                       bm25(jobs_fts, 1.0, 5.0, 1.0, 1.0), c.id
              LIMIT ?3 OFFSET ?4"
        ))
        .bind(any)
        .bind(country)
        .bind(limit)
        .bind(offset)
        .bind(all)
        .bind(any_word)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(Found {
                    job: row_to_job(r)?,
                    partial: !r.try_get::<bool, _>("all_words")?,
                })
            })
            .collect()
    }

    /// Active jobs with no cached role answer from `model`, one per distinct
    /// content (cache key, what the model should see), at most `limit`.
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

    /// Store model answers, then classify every active job again (a few
    /// thousand take milliseconds). Returns how many jobs' role changed.
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

    /// Every source that has ever stored a listing.
    /// Jobs by id, active or closed, in no particular order; unknown ids are
    /// simply absent.
    pub async fn get(&self, ids: &[String]) -> anyhow::Result<Vec<Job>> {
        let rows = sqlx::query(concat!(
            "SELECT c.*, ",
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

    /// Raw numbers for `sources eval`; the api layer shapes them.
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
    /// Active canonical jobs.
    pub canonical_jobs: i64,
    pub closed_jobs: i64,
    /// Live listings.
    pub source_jobs: i64,
    pub per_source: Vec<SourceRow>,
    /// (source a, source b, canonical jobs both list)
    pub overlaps: Vec<(String, String, i64)>,
    /// (job id, "title — company", "source=scope, …")
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

// ---------- dedup (IO side; the rules are in `dedup`) ----------

/// Re-cluster the live listings sharing `key` and carry out
/// [`dedup::cluster`]'s plan. Returns `false` when the key is ambiguous.
async fn recluster(
    conn: &mut SqliteConnection,
    key: &(String, String),
    ts: &str,
    rebuild: &mut BTreeSet<String>,
) -> anyhow::Result<bool> {
    let members: Vec<Member> = sqlx::query_as::<_, (String, String, String)>(
        "SELECT s.source_id, s.source_job_id, s.canonical_job_id FROM source_jobs s
         JOIN canonical_jobs c ON c.id = s.canonical_job_id
         WHERE s.company_key = ? AND s.title_key = ? AND s.job IS NOT NULL AND s.status = 'live'
         ORDER BY c.first_seen, c.id, s.source_id, s.source_job_id",
    )
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

/// Insert a canonical job seeded from one source job; `rebuild_canonical` fills it properly.
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

/// Recompute a canonical job from its listings (see [`dedup::resolve`]).
/// Deletes the job when nothing links to it any more.
async fn rebuild_canonical(conn: &mut SqliteConnection, cid: &str, ts: &str) -> anyhow::Result<()> {
    let rows: Vec<(String, String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT source_kind, source_id, source_job_id, job, status FROM source_jobs WHERE canonical_job_id = ?",
    )
    .bind(cid)
    .fetch_all(&mut *conn)
    .await?;
    if rows.is_empty() {
        sqlx::query("DELETE FROM jobs_fts WHERE job_id = ?")
            .bind(cid)
            .execute(&mut *conn)
            .await?;
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

    // The whole merged job is the stored record, so every field is in the
    // hash; status too, so a job that only opens or closes is still written.
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
    sqlx::query("DELETE FROM jobs_fts WHERE job_id = ?")
        .bind(cid)
        .execute(&mut *conn)
        .await?;
    sqlx::query("INSERT INTO jobs_fts (job_id, title, company, description) VALUES (?, ?, ?, ?)")
        .bind(cid)
        .bind(&m.title)
        .bind(&m.company)
        .bind(&m.description_md)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// A job's role as stored: `JobRole` JSON, or NULL when nothing places it.
/// Uses the model's cached answer for this job's content, if any.
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

// ---------- rows ----------

fn row_to_job(r: &sqlx::sqlite::SqliteRow) -> anyhow::Result<Job> {
    let parse_ts = |s: String| -> anyhow::Result<DateTime<Utc>> {
        Ok(DateTime::parse_from_rfc3339(&s)?.with_timezone(&Utc))
    };
    let mut sources: Vec<JobSource> = serde_json::from_str(&r.get::<String, _>("sources"))?;
    sources.sort_by(|a, b| a.source.cmp(&b.source));
    let m: NewJob = serde_json::from_str(&r.get::<String, _>("job"))?;
    // Parsed once for every reader, and only when a source field leaves one to read.
    let posting = (m.visa.is_none() || m.experience.is_none())
        .then(|| {
            m.description_md
                .as_deref()
                .map(crate::posting::Posting::parse)
        })
        .flatten();
    Ok(Job {
        id: r.get("id"),
        // Derived when read, so old rows get it without a re-fetch.
        seniority: crate::normalize::seniority(m.seniority.as_deref(), &m.title),
        title: m.title,
        company: m.company,
        employment_type: m.employment_type,
        eligibility: m.eligibility,
        pay: m.pay,
        // A source that states sponsorship in a field of its own beats the
        // reading of its description.
        visa: m
            .visa
            .clone()
            .or_else(|| posting.as_ref().and_then(crate::visa_text::read)),
        experience: m
            .experience
            .clone()
            .or_else(|| posting.as_ref().and_then(crate::experience_text::read)),
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

/// Quote every word so user input can never break FTS5 syntax. All words
/// must match, or with `any_word` at least one (bm25 then ranks jobs that
/// match more of them first).
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
