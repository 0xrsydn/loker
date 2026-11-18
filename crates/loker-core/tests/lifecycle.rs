//! Liveness: listings close (absent from a complete fetch, expired, stale) and
//! reopen; a job is active exactly while one of its listings is live.
mod common;

use common::{closed_jobs, clusters, days, ingest, ingest_at, job, store, t0};
use loker_core::model::Scope;

fn board(ids: &[&str]) -> Vec<loker_core::model::NewJob> {
    ids.iter()
        .map(|id| {
            job(
                "ashby:acme",
                id,
                "acme",
                &format!("Role {id}"),
                Scope::Worldwide,
                0.9,
            )
        })
        .collect()
}

#[tokio::test]
async fn complete_fetch_closes_missing_listings_and_reopens_them() {
    let s = store("absent").await;
    ingest_at(&s, "ashby:acme", &board(&["a1", "a2", "a3"]), t0(), true).await;

    let st = ingest_at(&s, "ashby:acme", &board(&["a1", "a2"]), days(1), true).await;
    assert_eq!((st.closed, st.held), (1, 0));
    assert_eq!(clusters(&s).await.len(), 2);
    assert_eq!(closed_jobs(&s).await, 1);

    let st = ingest_at(&s, "ashby:acme", &board(&["a1", "a2", "a3"]), days(2), true).await;
    assert_eq!((st.reopened, st.closed), (1, 0));
    assert_eq!(clusters(&s).await.len(), 3);
    assert_eq!(closed_jobs(&s).await, 0);
}

#[tokio::test]
async fn truncated_fetch_closes_nothing() {
    let s = store("truncated").await;
    ingest_at(
        &s,
        "ashby:acme",
        &board(&["a1", "a2", "a3", "a4"]),
        t0(),
        true,
    )
    .await;

    let st = ingest_at(&s, "ashby:acme", &board(&["a1"]), days(1), true).await;
    assert_eq!((st.closed, st.held), (0, 3));
    let st = ingest_at(&s, "ashby:acme", &[], days(1), true).await;
    assert_eq!((st.closed, st.held), (0, 4));
    assert_eq!(clusters(&s).await.len(), 4);
}

#[tokio::test]
async fn closing_a_duplicate_unblocks_the_merge_in_any_order() {
    let gh = |ids: &[&str]| -> Vec<_> {
        ids.iter()
            .map(|id| {
                job(
                    "greenhouse:binance",
                    id,
                    "Binance",
                    "Legal Project Manager",
                    Scope::Unknown,
                    0.3,
                )
            })
            .collect()
    };
    let him = [job(
        "himalayas",
        "h1",
        "Binance",
        "Legal Project Manager",
        Scope::Worldwide,
        0.95,
    )];

    // Himalayas before the duplicate closes.
    let early = store("unblock-early").await;
    ingest(&early, "himalayas", &him).await;
    ingest_at(&early, "greenhouse:binance", &gh(&["g1", "g2"]), t0(), true).await;
    assert_eq!(
        clusters(&early).await.len(),
        3,
        "two postings of one title block the merge"
    );
    ingest_at(&early, "greenhouse:binance", &gh(&["g1"]), days(1), true).await;

    // Himalayas after it closed.
    let late = store("unblock-late").await;
    ingest_at(&late, "greenhouse:binance", &gh(&["g1", "g2"]), t0(), true).await;
    ingest_at(&late, "greenhouse:binance", &gh(&["g1"]), days(1), true).await;
    ingest_at(&late, "himalayas", &him, days(1), false).await;

    let (e, l) = (clusters(&early).await, clusters(&late).await);
    assert_eq!(e, l, "active jobs must not depend on order");
    assert_eq!(
        e.len(),
        1,
        "the remaining posting merges with the aggregator copy: {e:#?}"
    );
    assert_eq!(
        (closed_jobs(&early).await, closed_jobs(&late).await),
        (1, 1)
    );
}

#[tokio::test]
async fn expiry_and_staleness_close_listings() {
    let s = store("expiry").await;
    let mut expiring = job(
        "himalayas",
        "h1",
        "Acme",
        "Designer",
        Scope::Worldwide,
        0.95,
    );
    expiring.expires_at = Some(days(10));
    let mut merged_agg = job(
        "himalayas",
        "h2",
        "Acme",
        "Engineer",
        Scope::Worldwide,
        0.95,
    );
    merged_agg.expires_at = Some(days(10));
    ingest(&s, "himalayas", &[expiring, merged_agg]).await;
    ingest(
        &s,
        "jobicy",
        &[job(
            "jobicy",
            "j1",
            "Other",
            "Writer",
            Scope::Worldwide,
            0.8,
        )],
    )
    .await;
    let board = [job(
        "ashby:acme",
        "a1",
        "acme",
        "Engineer",
        Scope::Onsite,
        0.85,
    )];
    ingest_at(&s, "ashby:acme", &board, t0(), true).await;
    assert_eq!(clusters(&s).await.len(), 3);

    // Day 11: both Himalayas listings expired. The Engineer job stays active
    // through its live board listing; the Designer job closes.
    let st = ingest_at(&s, "ashby:acme", &board, days(11), true).await;
    assert_eq!(st.closed, 2);
    assert_eq!(clusters(&s).await.len(), 2);
    assert_eq!(closed_jobs(&s).await, 1);

    // Day 31: nobody has seen the Jobicy listing for 30 days.
    let st = ingest_at(&s, "ashby:acme", &board, days(31), true).await;
    assert_eq!(st.closed, 1);
    let left = clusters(&s).await;
    assert_eq!(left.len(), 1);
    assert!(
        left.iter()
            .all(|c| c == &vec!["ashby:acme https://ashby:acme.example/a1".to_string()])
    );
}
