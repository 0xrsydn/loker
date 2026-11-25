use loker_core::normalize::eligibility_from_text;
use loker_core::verdict::{VerdictKind, Viewer, eligible_for};

#[test]
fn location_text_to_verdict() {
    use VerdictKind::*;
    let viewer = Viewer {
        country: "ID".into(),
        tz: Some(7.0),
        accept_onsite_in: Vec::new(),
    };
    let cases: &[(&str, Option<bool>, VerdictKind)] = &[
        ("Indonesia; Malaysia", Some(true), Yes),
        ("Hong Kong; Singapore; Taiwan", Some(true), No),
        ("APAC", Some(true), Yes),
        ("Southeast Asia", Some(true), Yes),
        ("EMEA", Some(true), No),
        ("LATAM", Some(true), No),
        ("Europe; Indonesia", Some(true), Yes),
        ("Anywhere", Some(true), Yes),
        ("Remote", Some(true), Unknown),
        ("Hong Kong; Singapore; Taiwan", None, No),
        ("Indonesia; Malaysia; Philippines", None, Maybe),
        ("APAC; Hong Kong", None, Maybe),
        ("Berlin", None, No),
        ("Remote - Warsaw", Some(true), No),
        ("Jakarta", Some(true), Maybe),
        ("Remote, Colorado", Some(true), No),
        ("", None, Unknown),
        ("Singapore", Some(false), No),
        ("Idaho Falls, ID", None, No),
        ("Bandung, ID", None, Maybe),
    ];
    for &(text, remote, want) in cases {
        let e = eligibility_from_text(text, remote);
        let v = eligible_for(&e, &viewer);
        assert_eq!(
            v.kind, want,
            "{text:?} remote={remote:?}: {} ({e:?})",
            v.reason
        );
    }
}

#[test]
fn a_code_after_a_place_is_read_as_its_state_or_its_country() {
    let cases: &[(&str, &[&str])] = &[
        ("San Francisco, CA", &["US"]),
        ("Westminster, CO", &["US"]),
        ("Chicago, IL", &["US"]),
        ("Arlington, VA", &["US"]),
        ("Idaho Falls, ID", &["US"]),
        ("Indianapolis, IN", &["US"]),
        ("San Francisco, CA | New York City, NY", &["US"]),
        ("Austin, TX; Toronto, ON", &["CA", "US"]),
        ("Vancouver, BC", &["CA"]),
        ("London, ON", &["CA"]),
        ("Dublin, CA", &["US"]),
        ("Mumbai, MH", &["IN"]),
        ("Thane, MH, India", &["IN"]),
        ("Chennai, TN", &["IN"]),
        ("Perth, WA", &["AU"]),
        ("Campinas, SP", &["BR"]),
        ("Jakarta, ID", &["ID"]),
        ("Bandung, ID", &["ID"]),
        ("Berlin, DE", &["DE"]),
        ("Dublin, IE", &["IE"]),
        ("Islamabad, PK", &["PK"]),
        ("London, UK", &["GB"]),
        ("Toronto, CA", &["CA"]),
        ("Remote (US)", &["US"]),
        ("Remote, CA", &["CA"]),
        ("Europe, DE", &["DE"]),
    ];
    for &(text, want) in cases {
        let e = eligibility_from_text(text, None);
        assert_eq!(e.countries, want, "{text:?}: {e:?}");
    }
    let near_a_city = eligibility_from_text("New York, NY", Some(true));
    assert_eq!(
        near_a_city.evidence.as_deref(),
        Some("names a city, not where remote work may be done from")
    );
}
