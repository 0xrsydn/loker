//! Who may take a Workable job, as its fields say.
use loker_core::model::{Elsewhere, Scope};
use loker_sources::workable::transform;
use serde_json::json;

#[test]
fn workplace_and_locations_say_who_may_apply() {
    let job = |workplace: &str, locations: Vec<&str>| {
        transform(json!({
            "id": "j1", "url": "https://jobs.workable.com/view/abc/dev", "title": "Developer",
            "company": {"title": "Acme"}, "workplace": workplace, "locations": locations,
            "location": {"countryName": "Philippines"},
            "description": "<p>Build things.</p>", "requirementsSection": "<p>Five years of Rust.</p>",
            "employmentType": "Full-time", "created": "2026-11-19T22:02:49.341Z"
        }))
        .unwrap()
    };
    let remote = job("remote", vec!["TELECOMMUTE", "Philippines"]);
    assert_eq!(
        (
            remote.eligibility.scope(),
            remote.eligibility.countries.clone()
        ),
        (Scope::Countries, vec!["PH".to_string()])
    );
    assert!(
        remote
            .description_md
            .unwrap()
            .contains("Five years of Rust")
    );
    let state = job(
        "remote",
        vec!["TELECOMMUTE", "Milwaukee, Wisconsin, United States"],
    );
    assert_eq!(state.eligibility.countries, vec!["US".to_string()]);
    // No place but the marker: the country Workable gives.
    assert_eq!(
        job("remote", vec!["TELECOMMUTE"]).eligibility.countries,
        vec!["PH".to_string()]
    );
    assert_eq!(
        job("hybrid", vec!["Makati City, Metro Manila, Philippines"])
            .eligibility
            .scope(),
        Scope::Hybrid
    );
    assert_eq!(
        job("on_site", vec!["Manila"]).eligibility.scope(),
        Scope::Onsite
    );
}

/// A remote job's place is the employer's, not a limit: the description decides.
#[test]
fn the_description_settles_a_remote_place() {
    let job = |text: &str| {
        transform(json!({
            "id": "j1", "url": "https://jobs.workable.com/view/abc/dev", "title": "Developer",
            "company": {"title": "Acme"}, "workplace": "remote",
            "locations": ["TELECOMMUTE", "Berlin, Berlin, Germany"],
            "description": format!("<p>{text}</p>"),
        }))
        .unwrap()
        .eligibility
    };
    let named = job("We recruit remotely in EMEA.");
    assert_eq!(
        (named.scope(), named.regions),
        (Scope::Region, vec!["EMEA".to_string()])
    );
    assert_eq!(named.elsewhere, Elsewhere::Excluded);
    // A looser hint: Workable's place holds, everyone else may be open.
    for wider in [
        "Fully remote, based outside the US.",
        "<strong>Location:</strong> International",
        "Work from anywhere.",
        "You have the freedom to live wherever you like.",
    ] {
        let e = job(wider);
        assert_eq!(
            (e.countries, e.elsewhere),
            (vec!["DE".to_string()], Elsewhere::Open),
            "{wider}"
        );
    }
    assert_eq!(
        job("This remote position is open to candidates worldwide.").scope(),
        Scope::Worldwide
    );
    // Nothing said: Workable's place for its own people, unknown for the rest.
    let silent = job("Build things.");
    assert_eq!(
        (silent.countries, silent.elsewhere),
        (vec!["DE".to_string()], Elsewhere::NotStated)
    );
}
