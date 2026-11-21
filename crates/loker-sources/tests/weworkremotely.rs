//! Where a We Work Remotely posting may be done from, as its feed fields say.
use loker_core::model::Scope;
use loker_sources::weworkremotely::transform;
use serde_json::json;

#[test]
fn country_is_narrower_than_region() {
    let item = |region: &str, country: Option<&str>| {
        let mut v = json!({"link": "https://weworkremotely.com/remote-jobs/acme-dev", "title": "Acme: Dev", "region": region});
        if let Some(c) = country {
            v["country"] = json!(c);
        }
        transform(v).unwrap().eligibility
    };
    let cases: [(&str, Option<&str>, Scope, &[&str]); 5] = [
        (
            "Anywhere in the World",
            Some("🇨🇦 Canada and 🇺🇸 United States of America"),
            Scope::Countries,
            &["CA", "US"],
        ),
        (
            "Anywhere in the World",
            Some(
                "🇫🇷 France, 🇩🇪 Germany, and 🇬🇧 United Kingdom of Great Britain and Northern Ireland",
            ),
            Scope::Countries,
            &["DE", "FR", "GB"],
        ),
        (
            "California",
            Some("🇺🇸 United States of America"),
            Scope::Countries,
            &["US"],
        ),
        // The board's default says remote, not where.
        ("Anywhere in the World", None, Scope::Unknown, &[]),
        ("USA Only", None, Scope::Countries, &["US"]),
    ];
    for (region, country, scope, countries) in cases {
        let e = item(region, country);
        assert_eq!(
            (
                e.scope(),
                e.countries
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .as_slice()
            ),
            (scope, countries),
            "{region} / {country:?}"
        );
    }
}
