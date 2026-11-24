use loker_sources::{SOURCES, Source};

#[test]
fn only_boards_that_list_nothing_but_remote_jobs_are_remote_only() {
    let cases = [
        ("himalayas", true),
        ("jobicy", true),
        ("remoteok", true),
        ("weworkremotely", true),
        ("workable", true),
        ("ycombinator", false),
        ("greenhouse:acme", false),
        ("ashby:acme", false),
        ("lever:acme", false),
        ("recruitee:acme", false),
        ("smartrecruiters:acme", false),
    ];
    assert_eq!(cases.len(), SOURCES.len(), "a row for every source");
    for (id, remote_only) in cases {
        assert_eq!(
            Source::parse(id).unwrap().remote_only(),
            remote_only,
            "{id}: a remote-only board lets its descriptions name places for unknown-workplace jobs"
        );
    }
}

#[test]
fn a_remote_lever_job_in_a_place_no_rule_reads_takes_lever_s_country() {
    let lever = Source::parse("lever:acme").unwrap();
    let cases = [
        ("Massachusetts - Boston", "US", "countries"),
        ("Remote", "US", "unknown"),
        ("Bogota", "CO", "unknown"),
        ("Anywhere", "US", "worldwide"),
    ];
    for (location, country, scope) in cases {
        let raw = serde_json::json!({
            "id": "1", "text": "Engineer", "workplaceType": "remote", "country": country,
            "categories": { "location": location, "allLocations": [location] },
            "hostedUrl": "https://jobs.lever.co/acme/1", "description": "<p>Build.</p>"
        });
        let job = loker_sources::job_from_raw(&lever, raw, None, None).unwrap();
        let wire = serde_json::to_value(&job.eligibility).unwrap();
        assert_eq!(wire["scope"], scope, "{location}: {wire}");
    }
}

#[test]
fn a_smartrecruiters_work_mode_field_wins_over_the_location_flags() {
    let sr = Source::parse("smartrecruiters:acme").unwrap();
    let cases = [
        (true, None, "remote"),
        (true, Some(("Home based?", "Hybrid")), "hybrid"),
        (true, Some(("Job Location", "On-Site Waltham")), "onsite"),
        (false, Some(("Remote Eligible", "Yes")), "remote"),
        (false, Some(("Work Model", "Onsite")), "onsite"),
        (false, Some(("Brands", "Hybrid Co")), "onsite"),
    ];
    for (remote, field, workplace) in cases {
        let custom: Vec<_> = field
            .map(|(l, v)| serde_json::json!({ "fieldLabel": l, "valueLabel": v }))
            .into_iter()
            .collect();
        let raw = serde_json::json!({
            "id": "1", "name": "Engineer", "customField": custom,
            "location": { "city": "Boston", "country": "us", "remote": remote, "hybrid": false,
                          "fullLocation": "Boston, MA, United States" },
            "jobAd": { "sections": { "jobDescription": { "title": "", "text": "<p>Build.</p>" } } }
        });
        let job = loker_sources::job_from_raw(&sr, raw, None, None).unwrap();
        let wire = serde_json::to_value(&job.eligibility).unwrap();
        assert_eq!(wire["workplace"], workplace, "{field:?}: {wire}");
    }
}
