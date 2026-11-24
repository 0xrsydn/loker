mod common;

use std::collections::BTreeSet;

use common::{days, ingest, job, store};
use loker_core::api::{Hit, SearchParams, View, search_jobs};
use loker_core::model::{Elsewhere, Scope, Seniority};
use loker_core::verdict::VerdictKind;

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
        "all five worldwide jobs, despite 600 newer ineligible ones: 300 US-only \
         (dropped in SQL) and 300 on-site (kept by SQL, dropped by the verdict: \
         more than one read)"
    );
    assert_eq!(r.next_offset, None);
}

#[tokio::test]
async fn offset_pages_cover_every_result_once() {
    let s = store("pages").await;
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
    assert_eq!(
        (seen.len(), pages),
        (25, 3),
        "same posted date everywhere, so only the id tiebreak orders them"
    );
}

async fn ranked(s: &loker_core::store::Store, q: &str) -> Vec<String> {
    let p = SearchParams {
        q: q.into(),
        ..Default::default()
    };
    let r = search_jobs(s, p).await.unwrap();
    r.jobs
        .into_iter()
        .map(|h| match h {
            Hit::Full(h) if h.partial => format!("~{}", h.job.title),
            Hit::Full(h) => h.job.title,
            Hit::Compact(c) => c.title,
            Hit::Titles(t) => t.title,
        })
        .collect()
}

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

    assert_eq!(
        ranked(&s, "pipeline").await,
        ["Data Engineering Lead"],
        "stemming: \"pipeline\" finds \"pipelines\""
    );
    assert_eq!(
        ranked(&s, "engineer").await.len(),
        3,
        "stemming: \"engineer\" finds \"Engineering\""
    );

    assert_eq!(
        ranked(&s, "rust").await,
        ["Rust Engineer", "Platform Engineer"],
        "a title match outranks the same word repeated in a description"
    );

    assert_eq!(
        ranked(&s, "rust team").await,
        ["Rust Engineer", "~Platform Engineer"],
        "jobs with every word first, then jobs with some, marked partial"
    );
    assert_eq!(
        ranked(&s, "pipelines kotlin").await,
        ["~Data Engineering Lead"]
    );

    let p = SearchParams {
        q: "rust team".into(),
        limit: Some(1),
        view: View::Titles,
        ..Default::default()
    };
    let r = search_jobs(&s, p).await.unwrap();
    assert_eq!(
        (r.count, r.total, r.full_matches),
        (1, 2, 1),
        "`total` counts every match across pages; `fullMatches` those with every word"
    );
    assert!(r.next_offset.is_some());

    let p = |names: &[&str]| SearchParams {
        q: "engineer".into(),
        role: names
            .iter()
            .map(|n| serde_json::from_value(serde_json::json!(n)).unwrap())
            .collect(),
        ..Default::default()
    };
    let count = |r: loker_core::api::SearchResult| r.total;
    assert_eq!(
        count(search_jobs(&s, p(&["data_engineering"])).await.unwrap()),
        1
    );
    assert_eq!(
        count(search_jobs(&s, p(&["data"])).await.unwrap()),
        1,
        "a role filter takes in any level"
    );
    assert_eq!(
        count(search_jobs(&s, p(&["engineering"])).await.unwrap()),
        3,
        "data engineering also sits in engineering; the Rust Engineer is backend \
         (its title names Rust), the Platform Engineer infrastructure"
    );
    assert_eq!(
        count(search_jobs(&s, p(&["design", "founding"])).await.unwrap()),
        0
    );

    for (level, want) in [(Seniority::Lead, 1), (Seniority::Senior, 0)] {
        let p = SearchParams {
            q: "engineer".into(),
            seniority: vec![level],
            ..Default::default()
        };
        assert_eq!(
            search_jobs(&s, p).await.unwrap().count,
            want,
            "a level filter keeps only jobs at those levels: {level:?}"
        );
    }
}

#[tokio::test]
async fn experience_filter_keeps_jobs_that_state_no_years() {
    let s = store("experience").await;
    let mut jobs = Vec::new();
    for (id, desc) in [
        ("a", Some("- 2+ years of experience with Go")),
        ("b", Some("- 7+ years of experience with Go")),
        ("c", Some("Strong Go skills.")),
        ("d", None),
    ] {
        let mut j = job(
            "jobicy",
            id,
            "Co",
            &format!("Go Engineer {id}"),
            Scope::Worldwide,
            0.8,
        );
        j.description_md = desc.map(str::to_string);
        jobs.push(j);
    }
    jobs[3].experience = loker_core::experience_text::experience_from_field("Any (new grads ok)");
    ingest(&s, "jobicy", &jobs).await;
    let titles = |years: Option<u8>| {
        let s = &s;
        async move {
            let r = search_jobs(
                s,
                SearchParams {
                    experience_years: years,
                    view: View::Compact,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            let mut got: Vec<(String, Option<u8>)> = r
                .jobs
                .iter()
                .map(|h| match h {
                    Hit::Compact(c) => (c.title.clone(), c.min_years),
                    _ => unreachable!(),
                })
                .collect();
            got.sort();
            got
        }
    };
    let all = titles(None).await;
    assert_eq!(
        all.iter().map(|(_, y)| *y).collect::<Vec<_>>(),
        [Some(2), Some(7), None, Some(0)]
    );
    let three: Vec<String> = titles(Some(3)).await.into_iter().map(|(t, _)| t).collect();
    assert_eq!(three, ["Go Engineer a", "Go Engineer c", "Go Engineer d"]);
}

#[tokio::test]
async fn posted_within_keeps_recent_jobs_and_drops_undated_ones() {
    let s = store("posted").await;
    let now = chrono::Utc::now();
    let jobs: Vec<_> = [("new", Some(5)), ("old", Some(400)), ("undated", None)]
        .into_iter()
        .map(|(id, age)| {
            let mut j = job(
                "jobicy",
                id,
                "Co",
                &format!("Engineer {id}"),
                Scope::Worldwide,
                0.8,
            );
            j.posted_at = age.map(|d| now - chrono::Duration::days(d));
            j
        })
        .collect();
    ingest(&s, "jobicy", &jobs).await;
    let titles = |within: Option<u32>| {
        let s = &s;
        async move {
            let r = search_jobs(
                s,
                SearchParams {
                    posted_within_days: within,
                    view: View::Titles,
                    ..Default::default()
                },
            )
            .await?;
            let mut got: Vec<String> = r
                .jobs
                .iter()
                .map(|h| match h {
                    Hit::Titles(t) => t.title.clone(),
                    _ => unreachable!(),
                })
                .collect();
            got.sort();
            Ok::<_, loker_core::api::Error>(got)
        }
    };
    assert_eq!(titles(None).await.unwrap().len(), 3);
    assert_eq!(titles(Some(30)).await.unwrap(), ["Engineer new"]);
    assert_eq!(
        titles(Some(730)).await.unwrap(),
        ["Engineer new", "Engineer old"]
    );
    assert!(titles(Some(0)).await.is_err());
}

#[tokio::test]
async fn a_country_list_drops_only_jobs_that_exclude_everyone_else() {
    let s = store("elsewhere").await;
    let jobs: Vec<_> = [
        ("excluded", Elsewhere::Excluded),
        ("not_stated", Elsewhere::NotStated),
        ("open", Elsewhere::Open),
    ]
    .into_iter()
    .map(|(id, elsewhere)| {
        let mut j = job(
            "workable",
            id,
            "Co",
            &format!("Engineer {id}"),
            Scope::Countries,
            0.8,
        );
        j.eligibility.countries = vec!["US".into()];
        j.eligibility.elsewhere = elsewhere;
        j
    })
    .collect();
    ingest(&s, "workable", &jobs).await;

    for q in ["", "engineer"] {
        let p = SearchParams {
            q: q.into(),
            view: View::Titles,
            ..params(10, None)
        };
        let found: BTreeSet<_> = search_jobs(&s, p)
            .await
            .unwrap()
            .jobs
            .into_iter()
            .map(|h| match h {
                Hit::Titles(t) => (t.title, t.verdict),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            found,
            BTreeSet::from([
                ("Engineer not_stated".into(), Some(VerdictKind::Unknown)),
                ("Engineer open".into(), Some(VerdictKind::Maybe)),
            ]),
            "q={q:?}: a US list stops a viewer in ID only when it excludes everyone else"
        );
    }
}
