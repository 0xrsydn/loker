mod common;

use common::{days, ingest, job, store};
use loker_core::api;
use loker_core::model::{Scope, TzStrength, TzWindow};
use loker_core::normalize;

#[tokio::test]
async fn every_json_path_the_sql_reads_resolves() {
    let mut j = job(
        "himalayas",
        "h1",
        "Acme",
        "Backend Engineer",
        Scope::Countries,
        0.9,
    );
    j.eligibility.countries = vec!["ID".into()];
    j.eligibility.evidence = Some("unmapped: Atlantis".into());
    j.eligibility.tz = Some(TzWindow {
        from: 5.0,
        to: 9.0,
        strength: TzStrength::Required,
    });
    j.pay = normalize::pay(Some(4000.0), Some(6000.0), Some("USD"), Some("monthly"));
    j.posted_at = Some(days(-3));
    j.description_md = Some("Build things.".into());
    let s = store("json-paths").await;
    ingest(&s, "himalayas", &[j]).await;

    let eval = serde_json::to_value(api::sources_eval(&s).await.unwrap()).unwrap();
    let src = &eval["sources"][0];
    for field in [
        "scope",
        "timezone",
        "pay",
        "payUsd",
        "postedAt",
        "description",
    ] {
        assert_eq!(
            src["coverage"][field], 1.0,
            "sources_eval reads '$.eligibility.scope', '$.eligibility.tz', '$.pay', \
             '$.pay.usdMonth*', '$.postedAt', '$.descriptionMd': coverage.{field}: {src:#}"
        );
    }
    assert_eq!(
        src["locationUnmapped"], 1,
        "sources_eval reads '$.eligibility.evidence': {src:#}"
    );

    let stats = api::stats(&s).await.unwrap();
    assert_eq!(
        stats.by_scope[0].scope, "countries",
        "generated column `remote_scope` ('$.eligibility.scope')"
    );
    assert_eq!(
        s.search("", Some("ID"), 10, 0).await.unwrap().len(),
        1,
        "generated from '$.eligibility.countries'"
    );
    assert_eq!(s.search("", Some("US"), 10, 0).await.unwrap().len(), 0);

    let mut newer = job(
        "himalayas",
        "h2",
        "Initech",
        "Data Engineer",
        Scope::Worldwide,
        0.9,
    );
    newer.posted_at = Some(days(-1));
    ingest(&s, "himalayas", &[newer]).await;
    let titles: Vec<String> = s
        .search("", None, 10, 0)
        .await
        .unwrap()
        .into_iter()
        .map(|j| j.title)
        .collect();
    assert_eq!(
        titles,
        ["Data Engineer", "Backend Engineer"],
        "`posted_at` ('$.postedAt') orders an empty query newest first"
    );
}
