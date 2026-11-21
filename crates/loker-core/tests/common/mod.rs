//! Helpers for the offline store tests: hand-written jobs, a temp database and
//! a fixed clock the tests move by hand.
#![allow(dead_code)]
use std::collections::BTreeSet;

use chrono::{DateTime, Duration, TimeZone, Utc};
use loker_core::model::{Eligibility, IngestStats, Method, NewJob, Scope, SourceKind};
use loker_core::store::{Batch, SourceMeta, Store};

/// Active jobs as sets of "source url", comparable across databases (ids are not).
pub type Clusters = BTreeSet<Vec<String>>;

pub async fn store(name: &str) -> Store {
    let dir = std::env::temp_dir().join(format!("loker-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    Store::open(&dir.join("loker.db")).await.unwrap()
}

pub fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

pub fn days(n: i64) -> DateTime<Utc> {
    t0() + Duration::days(n)
}

pub fn job(
    source: &str,
    id: &str,
    company: &str,
    title: &str,
    scope: Scope,
    confidence: f64,
) -> NewJob {
    let url = format!("https://{source}.example/{id}");
    NewJob {
        source_id: source.into(),
        source_job_id: id.into(),
        source_url: url.clone(),
        canonical_url: url,
        apply_url: None,
        title: title.into(),
        company: company.into(),
        seniority: None,
        category: None,
        employment_type: None,
        eligibility: Eligibility {
            confidence,
            ..Eligibility::unknown(Method::Rules)
        }
        .with_scope(scope),
        pay: None,
        tech_tags: vec![],
        description_md: None,
        posted_at: None,
        expires_at: None,
        visa: None,
        raw: serde_json::Value::Null,
    }
}

fn meta(id: &str) -> SourceMeta {
    let kind = if id.contains(':') {
        SourceKind::Employer
    } else {
        SourceKind::Aggregator
    };
    SourceMeta {
        id: id.into(),
        kind,
    }
}

/// A partial feed (an aggregator's newest N) at `t0`.
pub async fn ingest(s: &Store, source: &str, jobs: &[NewJob]) -> IngestStats {
    ingest_at(s, source, jobs, t0(), false).await
}

/// `complete`: the batch is the source's whole list, so missing listings close.
pub async fn ingest_at(
    s: &Store,
    source: &str,
    jobs: &[NewJob],
    now: DateTime<Utc>,
    complete: bool,
) -> IngestStats {
    let complete_ids = complete.then(|| jobs.iter().map(|j| j.source_job_id.clone()).collect());
    s.ingest(Batch {
        source: meta(source),
        jobs: jobs.to_vec(),
        skipped: 0,
        complete_ids,
        now,
    })
    .await
    .unwrap()
}

pub async fn clusters(s: &Store) -> Clusters {
    s.search("", None, 10_000, 0)
        .await
        .unwrap()
        .into_iter()
        .map(|j| {
            j.sources
                .into_iter()
                .map(|x| format!("{} {}", x.source, x.url))
                .collect()
        })
        .collect()
}

pub async fn closed_jobs(s: &Store) -> i64 {
    loker_core::api::sources_eval(s).await.unwrap().closed_jobs
}
