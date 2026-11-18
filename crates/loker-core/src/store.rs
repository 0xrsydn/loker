//! SQLite store: one file, WAL mode, migrations embedded at compile time.
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};

use crate::model::{IngestStats, Job, NewJob};

pub struct Store {
    pool: SqlitePool,
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
        Ok(Self { pool })
    }

    /// Upsert one source's batch in a single transaction. Re-ingesting identical
    /// content only touches `last_seen`, so running ingest twice is a no-op.
    pub async fn ingest(
        &self,
        source_id: &str,
        jobs: Vec<NewJob>,
        skipped: u32,
    ) -> anyhow::Result<IngestStats> {
        let started = now();
        let mut stats = IngestStats {
            fetched: jobs.len() as u32 + skipped,
            skipped,
            ..Default::default()
        };
        let mut tx = self.pool.begin().await?;
        for job in jobs {
            let id = job_id(&job.canonical_url);
            let eligibility = serde_json::to_string(&job.eligibility)?;
            let pay = job.pay.as_ref().map(serde_json::to_string).transpose()?;
            let tags = serde_json::to_string(&job.tech_tags)?;
            let posted = job.posted_at.map(|t| t.to_rfc3339());
            let hash = content_hash(&[
                &job.title,
                &job.company,
                &eligibility,
                pay.as_deref().unwrap_or(""),
                &tags,
                job.description_md.as_deref().unwrap_or(""),
                job.apply_url.as_deref().unwrap_or(""),
                posted.as_deref().unwrap_or(""),
                job.seniority.as_deref().unwrap_or(""),
                job.employment_type.as_deref().unwrap_or(""),
            ]);
            let ts = now();
            let existing: Option<String> =
                sqlx::query_scalar("SELECT content_hash FROM canonical_jobs WHERE id = ?")
                    .bind(&id)
                    .fetch_optional(&mut *tx)
                    .await?;
            match existing {
                Some(h) if h == hash => {
                    sqlx::query(
                        "UPDATE canonical_jobs SET last_seen = ?, status = 'active' WHERE id = ?",
                    )
                    .bind(&ts)
                    .bind(&id)
                    .execute(&mut *tx)
                    .await?;
                    stats.unchanged += 1;
                }
                existing => {
                    sqlx::query(
                        "INSERT INTO canonical_jobs (id, canonical_url, apply_url, title, company, seniority,
                           employment_type, eligibility, pay, tech_tags, description_md, content_hash,
                           posted_at, first_seen, last_seen, status)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14, 'active')
                         ON CONFLICT (id) DO UPDATE SET
                           apply_url = excluded.apply_url, title = excluded.title, company = excluded.company,
                           seniority = excluded.seniority, employment_type = excluded.employment_type,
                           eligibility = excluded.eligibility, pay = excluded.pay, tech_tags = excluded.tech_tags,
                           description_md = excluded.description_md, content_hash = excluded.content_hash,
                           posted_at = excluded.posted_at, last_seen = excluded.last_seen, status = 'active'",
                    )
                    .bind(&id).bind(&job.canonical_url).bind(&job.apply_url).bind(&job.title)
                    .bind(&job.company).bind(&job.seniority).bind(&job.employment_type).bind(&eligibility)
                    .bind(&pay).bind(&tags).bind(&job.description_md).bind(&hash).bind(&posted).bind(&ts)
                    .execute(&mut *tx)
                    .await?;
                    sqlx::query("DELETE FROM jobs_fts WHERE job_id = ?")
                        .bind(&id)
                        .execute(&mut *tx)
                        .await?;
                    sqlx::query("INSERT INTO jobs_fts (job_id, title, company, description) VALUES (?, ?, ?, ?)")
                        .bind(&id).bind(&job.title).bind(&job.company).bind(&job.description_md)
                        .execute(&mut *tx)
                        .await?;
                    if existing.is_some() {
                        stats.updated += 1
                    } else {
                        stats.new += 1
                    }
                }
            }
            sqlx::query(
                "INSERT INTO source_jobs (source_id, source_job_id, source_url, canonical_job_id, raw_data, fetched_at)
                 VALUES (?, ?, ?, ?, ?, ?)
                 ON CONFLICT (source_id, source_job_id) DO UPDATE SET
                   source_url = excluded.source_url, canonical_job_id = excluded.canonical_job_id,
                   raw_data = excluded.raw_data, fetched_at = excluded.fetched_at",
            )
            .bind(&job.source_id).bind(&job.source_job_id).bind(&job.source_url).bind(&id)
            .bind(job.raw.to_string()).bind(&ts)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "INSERT INTO ingestion_runs (source_id, started_at, completed_at, status, jobs_fetched, jobs_new,
               jobs_updated, jobs_unchanged, jobs_skipped) VALUES (?, ?, ?, 'completed', ?, ?, ?, ?, ?)",
        )
        .bind(source_id).bind(&started).bind(now()).bind(stats.fetched).bind(stats.new)
        .bind(stats.updated).bind(stats.unchanged).bind(stats.skipped)
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

    /// Full-text search over active jobs (every word must match), best match first.
    /// An empty query lists the newest jobs.
    pub async fn search(&self, query: &str, limit: u32) -> anyhow::Result<Vec<Job>> {
        let rows = match fts_query(query) {
            Some(q) => {
                sqlx::query(
                    "SELECT c.* FROM jobs_fts JOIN canonical_jobs c ON c.id = jobs_fts.job_id
                     WHERE jobs_fts MATCH ? AND c.status = 'active'
                     ORDER BY bm25(jobs_fts) LIMIT ?",
                )
                .bind(q).bind(limit).fetch_all(&self.pool).await?
            }
            None => {
                sqlx::query("SELECT * FROM canonical_jobs WHERE status = 'active' ORDER BY posted_at DESC LIMIT ?")
                    .bind(limit).fetch_all(&self.pool).await?
            }
        };
        rows.iter().map(row_to_job).collect()
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
}

fn row_to_job(r: &sqlx::sqlite::SqliteRow) -> anyhow::Result<Job> {
    let parse_ts = |s: Option<String>| -> anyhow::Result<Option<DateTime<Utc>>> {
        s.map(|s| Ok(DateTime::parse_from_rfc3339(&s)?.with_timezone(&Utc)))
            .transpose()
    };
    Ok(Job {
        id: r.get("id"),
        title: r.get("title"),
        company: r.get("company"),
        seniority: r.get("seniority"),
        employment_type: r.get("employment_type"),
        eligibility: serde_json::from_str(&r.get::<String, _>("eligibility"))?,
        pay: r
            .get::<Option<String>, _>("pay")
            .map(|p| serde_json::from_str(&p))
            .transpose()?,
        tech_tags: serde_json::from_str(&r.get::<String, _>("tech_tags"))?,
        url: r
            .get::<Option<String>, _>("apply_url")
            .unwrap_or_else(|| r.get("canonical_url")),
        posted_at: parse_ts(r.get("posted_at"))?,
        first_seen: parse_ts(Some(r.get("first_seen")))?.expect("first_seen is NOT NULL"),
        status: r.get("status"),
    })
}

/// Quote every word so user input can never break FTS5 syntax.
fn fts_query(q: &str) -> Option<String> {
    let terms: Vec<String> = q
        .split_whitespace()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

pub fn job_id(canonical_url: &str) -> String {
    format!(
        "j_{}",
        &blake3::hash(canonical_url.as_bytes()).to_hex()[..10]
    )
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
