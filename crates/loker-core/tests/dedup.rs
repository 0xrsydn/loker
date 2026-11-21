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
        let merged = s.search("expert", None, 10, 0).await.unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].url, "https://ashby:clera.example/a1");
        assert_eq!(merged[0].eligibility.scope(), Scope::Worldwide);
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

/// One board says "Anywhere", another "Canada": the narrower statement wins,
/// whichever is more confident.
#[test]
fn places_beat_worldwide_in_a_merge() {
    use loker_core::dedup::{Linked, resolve};
    use loker_core::model::SourceKind;
    let mut canada = job("jobicy", "j1", "Acme", "Dev", Scope::Countries, 0.75);
    canada.eligibility.countries = vec!["CA".into()];
    let anywhere = job("weworkremotely", "w1", "Acme", "Dev", Scope::Worldwide, 0.8);
    let linked = |j: loker_core::model::NewJob| Linked {
        kind: SourceKind::Aggregator,
        source_id: j.source_id.clone(),
        source_job_id: j.source_job_id.clone(),
        job: Some(j),
        live: true,
    };
    let e = resolve(vec![linked(anywhere), linked(canada)])
        .unwrap()
        .job
        .eligibility;
    assert_eq!(
        (e.scope(), e.countries),
        (Scope::Countries, vec!["CA".to_string()])
    );
}

#[test]
fn cluster_rules() {
    use loker_core::dedup::{Member, Plan, cluster};
    // Rows as the store hands them over: oldest canonical job first.
    let m = |source: &str, id: &str, canonical: &str| Member {
        source_id: source.into(),
        source_job_id: id.into(),
        canonical_id: canonical.into(),
    };
    let key = ("acme".to_string(), "backend engineer".to_string());
    let no_company = (String::new(), "backend engineer".to_string());
    let merge = |into: &str, moves: &[usize]| Plan::Merge {
        into: into.into(),
        moves: moves.to_vec(),
    };
    let cases = [
        (
            "one listing",
            &key,
            vec![m("himalayas", "h1", "j_a")],
            Plan::Keep,
        ),
        (
            "empty key never merges",
            &no_company,
            vec![m("himalayas", "h1", "j_a"), m("jobicy", "j1", "j_b")],
            Plan::Keep,
        ),
        (
            "one source repeating a title is not ambiguous",
            &key,
            vec![
                m("greenhouse:acme", "g1", "j_a"),
                m("greenhouse:acme", "g2", "j_b"),
            ],
            Plan::Keep,
        ),
        (
            "one per source merges onto the oldest canonical job",
            &key,
            vec![
                m("himalayas", "h1", "j_a"),
                m("jobicy", "j1", "j_b"),
                m("remoteok", "r1", "j_a"),
            ],
            merge("j_a", &[1]),
        ),
        (
            "already merged: nothing moves",
            &key,
            vec![m("himalayas", "h1", "j_a"), m("jobicy", "j1", "j_a")],
            merge("j_a", &[]),
        ),
        (
            "a repeated source splits everyone sharing a canonical job",
            &key,
            vec![
                m("himalayas", "h1", "j_a"),
                m("greenhouse:acme", "g1", "j_a"),
                m("greenhouse:acme", "g2", "j_b"),
                m("remoteok", "r1", "j_a"),
            ],
            Plan::Split { evict: vec![1, 3] },
        ),
    ];
    for (name, key, members, want) in cases {
        assert_eq!(cluster(key, &members), want, "{name}");
    }
}
