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

fn varied_jobs(n: usize) -> Vec<loker_core::model::NewJob> {
    use loker_core::model::{Elsewhere, TzStrength, TzWindow, Workplace};
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = move |m: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % m as u64) as usize
    };
    let titles = [
        "Senior Backend Engineer",
        "Junior Data Analyst",
        "Staff Machine Learning Engineer",
        "Account Executive",
        "Engineering Intern",
        "Forward Deployed Engineer",
        "Product Designer",
        "Lead Data Engineer",
    ];
    let texts = [
        None,
        Some("We need 3+ years of experience with Go."),
        Some("At least 7 years of experience building data systems."),
        Some("We sponsor visas. 2+ years of experience required."),
    ];
    let now = chrono::Utc::now();
    (0..n)
        .map(|i| {
            let mut j = job(
                "jobicy",
                &format!("p{i}"),
                ["Acme", "Globex", "Initech"][next(3)],
                &format!("{} {i}", titles[next(titles.len())]),
                Scope::Unknown,
                0.8,
            );
            let e = &mut j.eligibility;
            e.workplace = [
                Workplace::Remote,
                Workplace::Hybrid,
                Workplace::Onsite,
                Workplace::Unknown,
            ][next(4)];
            e.elsewhere = [Elsewhere::Excluded, Elsewhere::NotStated, Elsewhere::Open][next(3)];
            e.countries = ["US", "ID", "DE", "BR"]
                .iter()
                .filter(|_| next(3) == 0)
                .map(|c| c.to_string())
                .collect();
            e.regions = ["Europe", "APAC", "Atlantis", "LATAM"]
                .iter()
                .filter(|_| next(4) == 0)
                .map(|r| r.to_string())
                .collect();
            e.tz = (next(3) == 0).then(|| TzWindow {
                from: [-8.0, 0.0, 5.0][next(3)],
                to: [2.0, 9.0, 12.0][next(3)],
                strength: if next(2) == 0 {
                    TzStrength::Required
                } else {
                    TzStrength::Preferred
                },
            });
            j.description_md = texts[next(texts.len())].map(str::to_string);
            j.posted_at = [None, Some(2), Some(40), Some(400)][next(4)]
                .map(|d| now - chrono::Duration::days(d));
            j
        })
        .collect()
}

async fn every_hit(s: &loker_core::store::Store, p: &SearchParams) -> (u32, u32, Vec<String>) {
    let mut ids = Vec::new();
    let mut offset = 0;
    loop {
        let r = search_jobs(
            s,
            SearchParams {
                limit: Some(100),
                offset: Some(offset),
                view: View::Titles,
                ..p.clone()
            },
        )
        .await
        .unwrap();
        ids.extend(r.jobs.iter().map(|h| h.id().to_string()));
        match r.next_offset {
            Some(o) => offset = o,
            None => return (r.total, r.full_matches, ids),
        }
    }
}

#[tokio::test]
async fn sql_filters_keep_exactly_the_jobs_the_rules_keep() {
    use loker_core::api::MinVerdict;
    use loker_core::role::{Modifier, Role, RoleFilter};
    use loker_core::verdict::{Viewer, eligible_for};
    let s = store("parity").await;
    ingest(&s, "jobicy", &varied_jobs(300)).await;
    let jobs: std::collections::HashMap<String, loker_core::model::Job> = {
        let (_, _, ids) = every_hit(&s, &SearchParams::default()).await;
        assert_eq!(ids.len(), 300);
        s.get(&ids)
            .await
            .unwrap()
            .into_iter()
            .map(|j| (j.id.clone(), j))
            .collect()
    };
    let mut seed: u64 = 7;
    let mut next = move |m: usize| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((seed >> 33) % m as u64) as usize
    };
    let mut narrowed = 0;
    let mut by_query: std::collections::HashMap<(String, bool), Vec<String>> = Default::default();
    for _ in 0..100 {
        let country = [None, Some("ID"), Some("DE"), Some("US"), Some("BR")][next(5)];
        let p = SearchParams {
            q: ["", "engineer", "data analyst", "designer intern"][next(4)].into(),
            any_word: next(4) == 0,
            country: country.map(str::to_string),
            tz: country.and([None, Some(7.0), Some(-5.0)][next(3)]),
            min_verdict: country
                .and([None, Some(MinVerdict::Maybe), Some(MinVerdict::Yes)][next(3)]),
            seniority: [
                vec![],
                vec![Seniority::Senior],
                vec![Seniority::Junior, Seniority::Intern],
            ][next(3)]
            .clone(),
            experience_years: [None, Some(3)][next(2)],
            posted_within_days: [None, Some(30), Some(365)][next(3)],
            role: [
                vec![],
                vec![RoleFilter::Role(Role::Engineering)],
                vec![
                    RoleFilter::Role(Role::Data),
                    RoleFilter::Modifier(Modifier::ForwardDeployed),
                ],
            ][next(3)]
            .clone(),
            ..Default::default()
        };
        let key = (p.q.clone(), p.any_word);
        if !by_query.contains_key(&key) {
            let (_, _, ids) = every_hit(
                &s,
                &SearchParams {
                    q: p.q.clone(),
                    any_word: p.any_word,
                    ..Default::default()
                },
            )
            .await;
            by_query.insert(key.clone(), ids);
        }
        let matching = by_query[&key].clone();
        let viewer = p.country.as_ref().map(|c| Viewer {
            country: c.clone(),
            tz: p.tz,
            accept_onsite_in: vec![],
        });
        let worst = match p.min_verdict {
            Some(MinVerdict::Yes) => VerdictKind::Yes,
            Some(MinVerdict::Maybe) => VerdictKind::Maybe,
            None => VerdictKind::Unknown,
        };
        let after = p
            .posted_within_days
            .map(|d| chrono::Utc::now() - chrono::Duration::days(i64::from(d)));
        let matched = matching.len();
        let expected: Vec<String> = matching
            .into_iter()
            .filter(|id| {
                let j = &jobs[id];
                viewer
                    .as_ref()
                    .is_none_or(|v| eligible_for(&j.eligibility, v).kind <= worst)
                    && (p.role.is_empty() || j.role.as_ref().is_some_and(|r| r.matches(&p.role)))
                    && (p.seniority.is_empty()
                        || j.seniority.is_some_and(|l| p.seniority.contains(&l)))
                    && p.experience_years
                        .zip(j.experience.as_ref())
                        .is_none_or(|(y, e)| e.min_years <= y)
                    && after.is_none_or(|a| j.posted_at.is_some_and(|t| t >= a))
            })
            .collect();
        let (total, _, got) = every_hit(&s, &p).await;
        assert_eq!(got, expected, "{p:?}");
        assert_eq!(total as usize, expected.len(), "{p:?}");
        narrowed += usize::from(!expected.is_empty() && expected.len() < matched);
    }
    assert!(
        narrowed >= 30,
        "only {narrowed} searches were narrowed by a filter"
    );
}

#[tokio::test]
async fn rows_derived_by_older_rules_are_derived_again_on_open() {
    let s = store("restamp").await;
    let mut j = job(
        "jobicy",
        "r1",
        "Co",
        "Senior Go Engineer",
        Scope::Worldwide,
        0.8,
    );
    j.description_md = Some("- 5+ years of experience with Go".into());
    ingest(&s, "jobicy", &[j]).await;
    drop(s);
    let path = std::env::temp_dir()
        .join(format!("loker-test-{}-restamp", std::process::id()))
        .join("loker.db");
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    sqlx::query("UPDATE canonical_jobs SET derived_stamp = 0, seniority = NULL, min_years = NULL, experience = NULL")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let s = loker_core::store::Store::open(&path).await.unwrap();
    let r = search_jobs(
        &s,
        SearchParams {
            seniority: vec![Seniority::Senior],
            experience_years: Some(4),
            view: View::Compact,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(r.total, 0);
    let r = search_jobs(
        &s,
        SearchParams {
            seniority: vec![Seniority::Senior],
            view: View::Compact,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    match &r.jobs[..] {
        [Hit::Compact(c)] => assert_eq!(c.min_years, Some(5)),
        other => panic!("{other:?}"),
    }
}
