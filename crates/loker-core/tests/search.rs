//! Search correctness: eligible jobs ranked below many ineligible ones are
//! still found, and offset pages neither skip nor repeat.
mod common;

use std::collections::BTreeSet;

use common::{days, ingest, job, store};
use loker_core::api::{Hit, SearchParams, search_jobs};
use loker_core::model::Scope;

fn params(limit: u32, offset: Option<u32>) -> SearchParams {
    SearchParams {
        q: String::new(),
        country: Some("ID".into()),
        tz: Some(7.0),
        limit: Some(limit),
        offset,
        ..Default::default()
    }
}

#[tokio::test]
async fn eligible_jobs_behind_many_ineligible_ones_are_found() {
    let s = store("buried").await;
    let mut jobs = Vec::new();
    // 600 newer jobs the viewer can't take: 300 US-only (dropped in SQL) and
    // 300 on-site (kept by SQL, dropped by the verdict: more than one read).
    for i in 0..600 {
        let scope = if i % 2 == 0 {
            Scope::Countries
        } else {
            Scope::Onsite
        };
        let mut j = job(
            "jobicy",
            &format!("x{i}"),
            "Co",
            &format!("Role {i}"),
            scope,
            0.8,
        );
        j.eligibility.countries = vec!["US".into()];
        j.posted_at = Some(days(10));
        jobs.push(j);
    }
    for i in 0..5 {
        let mut j = job(
            "jobicy",
            &format!("w{i}"),
            "Co",
            &format!("Remote {i}"),
            Scope::Worldwide,
            0.8,
        );
        j.posted_at = Some(days(1));
        jobs.push(j);
    }
    ingest(&s, "jobicy", &jobs).await;

    let r = search_jobs(&s, params(10, None)).await.unwrap();
    assert_eq!(
        r.count, 5,
        "all five worldwide jobs, despite 600 newer ineligible ones"
    );
    assert_eq!(r.next_offset, None);
}

#[tokio::test]
async fn offset_pages_cover_every_result_once() {
    let s = store("pages").await;
    // Same posted date everywhere, so only the id tiebreak orders them.
    let jobs: Vec<_> = (0..25)
        .map(|i| {
            let mut j = job(
                "jobicy",
                &format!("w{i}"),
                "Co",
                &format!("Remote {i}"),
                Scope::Worldwide,
                0.8,
            );
            j.posted_at = Some(days(1));
            j
        })
        .collect();
    ingest(&s, "jobicy", &jobs).await;

    let (mut seen, mut offset, mut pages) = (BTreeSet::new(), None, 0);
    loop {
        let r = search_jobs(&s, params(10, offset)).await.unwrap();
        pages += 1;
        for hit in &r.jobs {
            assert!(seen.insert(hit.id().to_string()), "{} repeated", hit.id());
        }
        match r.next_offset {
            Some(n) => offset = Some(n),
            None => break,
        }
    }
    assert_eq!((seen.len(), pages), (25, 3));
}

/// Titles of the jobs a query finds, best first.
async fn ranked(s: &loker_core::store::Store, q: &str) -> Vec<String> {
    let p = SearchParams {
        q: q.into(),
        ..Default::default()
    };
    let r = search_jobs(s, p).await.unwrap();
    r.jobs
        .into_iter()
        .map(|h| match h {
            Hit::Full(h) => h.job.title,
            Hit::Compact(c) => c.title,
        })
        .collect()
}

/// The ranking rules eval/search measured, one case each.
#[tokio::test]
async fn ranking() {
    let s = store("ranking").await;
    let with = |id: &str, title: &str, text: &str| {
        let mut j = job("jobicy", id, "Co", title, Scope::Worldwide, 0.8);
        j.description_md = Some(text.into());
        j
    };
    let jobs = [
        with("lead", "Data Engineering Lead", "Own the pipelines."),
        with(
            "rust",
            "Rust Engineer",
            "Build backend services in a small team.",
        ),
        with(
            "platform",
            "Platform Engineer",
            "Rust services. Rust tooling. Some Rust.",
        ),
    ];
    ingest(&s, "jobicy", &jobs).await;

    // Stemming: "engineer" and "pipeline" find "Engineering" and "pipelines".
    assert_eq!(ranked(&s, "pipeline").await, ["Data Engineering Lead"]);
    assert_eq!(ranked(&s, "engineer").await.len(), 3);

    // A title match outranks the same word repeated in a description.
    assert_eq!(
        ranked(&s, "rust").await,
        ["Rust Engineer", "Platform Engineer"]
    );
}
