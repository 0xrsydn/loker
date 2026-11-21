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
