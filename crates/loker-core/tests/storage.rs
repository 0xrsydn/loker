//! SQL reads fields out of the stored job JSON by name (`'$.eligibility.scope'`,
//! `'$.descriptionMd'`, …). A renamed Rust field would make those reads come
//! back NULL instead of failing, so pin each one through the numbers it feeds.
mod common;

use common::{days, ingest, job, store};
use loker_core::api;
use loker_core::model::{Pay, Scope, TzStrength, TzWindow};

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
    j.pay = Some(Pay {
        usd_month_min: Some(4000),
        usd_month_max: Some(6000),
        raw: "$4k-6k/mo".into(),
    });
    j.posted_at = Some(days(-3));
    j.description_md = Some("Build things.".into());
    let s = store("json-paths").await;
    ingest(&s, "himalayas", &[j]).await;

    // sources_eval: '$.eligibility.{scope,evidence,tz}', '$.pay', '$.pay.usdMonth*',
    // '$.postedAt', '$.descriptionMd'.
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
        assert_eq!(src["coverage"][field], 1.0, "coverage.{field}: {src:#}");
    }
    assert_eq!(src["locationUnmapped"], 1, "{src:#}");

    // The generated `remote_scope` column ('$.scope') and '$.countries' in search.
    let stats = api::stats(&s).await.unwrap();
    assert_eq!(stats.by_scope[0].scope, "countries");
    assert_eq!(s.search("", Some("ID"), 10, 0).await.unwrap().len(), 1);
    assert_eq!(s.search("", Some("US"), 10, 0).await.unwrap().len(), 0);
}
