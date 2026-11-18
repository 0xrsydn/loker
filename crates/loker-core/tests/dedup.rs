//! Cross-source dedup rules, offline: hand-written jobs into a temp database.
//! Clusters are compared as sets of (source, url), never by id, because ids
//! legitimately depend on which source arrived first.
mod common;

use std::collections::BTreeSet;

use common::{clusters, ingest, job, store};
use loker_core::model::Scope;

#[tokio::test]
async fn merge_is_order_independent_and_idempotent() {
    // The same role on an aggregator, the employer's board and a second aggregator,
    // spelled the way each really spells it.
    let him = [
        job(
            "himalayas",
            "h1",
            "Clera",
            "Expert Talent Pool, Physical AI",
            Scope::Worldwide,
            0.95,
        ),
        job(
            "himalayas",
            "h2",
            "Clera",
            "Data Operations Associate",
            Scope::Worldwide,
            0.95,
        ),
    ];
    let ats = [
        job(
            "ashby:clera",
            "a1",
            "clera",
            "Expert Talent Pool Physical AI",
            Scope::Onsite,
            0.85,
        ),
        job(
            "ashby:clera",
            "a2",
            "clera",
            "Research Engineer",
            Scope::Onsite,
            0.85,
        ),
    ];
    let rok = [job(
        "remoteok",
        "r1",
        "Clera",
        "Expert Talent Pool Physical AI",
        Scope::Unknown,
        0.3,
    )];

    let forward = store("forward").await;
    ingest(&forward, "himalayas", &him).await;
    ingest(&forward, "ashby:clera", &ats).await;
    let last = ingest(&forward, "remoteok", &rok).await;
    assert_eq!(last.merged, 1);

    let backward = store("backward").await;
    ingest(&backward, "remoteok", &rok).await;
    ingest(&backward, "ashby:clera", &ats).await;
    ingest(&backward, "himalayas", &him).await;

    let (f, b) = (clusters(&forward).await, clusters(&backward).await);
    assert_eq!(f, b, "dedup must not depend on ingest order");
    assert_eq!(
        f.len(),
        3,
        "three listings of one role become one job: {f:#?}"
    );

    // Employer board supplies identity; the most confident source supplies eligibility.
    for s in [&forward, &backward] {
        let merged = s.search("expert", 10).await.unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].url, "https://ashby:clera.example/a1");
        assert_eq!(merged[0].eligibility.scope, Scope::Worldwide);
    }

    let again = ingest(&forward, "himalayas", &him).await;
    assert_eq!((again.new, again.updated, again.unchanged), (0, 0, 2));
}

#[tokio::test]
async fn same_source_duplicates_block_the_merge() {
    // Binance posts one title per city; a single aggregator copy can't be matched to either.
    let gh = [
        job(
            "greenhouse:binance",
            "g1",
            "Binance",
            "Legal Project Manager",
            Scope::Unknown,
            0.3,
        ),
        job(
            "greenhouse:binance",
            "g2",
            "Binance",
            "Legal Project Manager",
            Scope::Unknown,
            0.3,
        ),
    ];
    let him = [job(
        "himalayas",
        "h1",
        "Binance",
        "Legal Project Manager",
        Scope::Worldwide,
        0.95,
    )];
    let rok = [job(
        "remoteok",
        "r1",
        "Binance",
        "Legal Project Manager",
        Scope::Unknown,
        0.3,
    )];

    // Every order of three sources gives the same answer: four separate jobs.
    let mut seen = BTreeSet::new();
    for (i, order) in [["h", "r", "g"], ["g", "h", "r"], ["r", "g", "h"]]
        .iter()
        .enumerate()
    {
        let s = store(&format!("blocked{i}")).await;
        for o in order {
            match *o {
                "h" => ingest(&s, "himalayas", &him).await,
                "r" => ingest(&s, "remoteok", &rok).await,
                _ => ingest(&s, "greenhouse:binance", &gh).await,
            };
        }
        let c = clusters(&s).await;
        assert_eq!(c.len(), 4, "order {order:?}: {c:#?}");
        seen.insert(c);
    }
    assert_eq!(seen.len(), 1);
}

#[tokio::test]
async fn retitled_job_leaves_its_cluster() {
    let s = store("retitle").await;
    ingest(
        &s,
        "himalayas",
        &[job(
            "himalayas",
            "h1",
            "Acme",
            "Backend Engineer",
            Scope::Worldwide,
            0.95,
        )],
    )
    .await;
    ingest(
        &s,
        "jobicy",
        &[job(
            "jobicy",
            "j1",
            "Acme Inc.",
            "Backend Engineer",
            Scope::Worldwide,
            0.8,
        )],
    )
    .await;
    assert_eq!(clusters(&s).await.len(), 1);

    let st = ingest(
        &s,
        "jobicy",
        &[job(
            "jobicy",
            "j1",
            "Acme Inc.",
            "Platform Engineer",
            Scope::Worldwide,
            0.8,
        )],
    )
    .await;
    assert_eq!(st.updated, 1);
    assert_eq!(clusters(&s).await.len(), 2);
}
