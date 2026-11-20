//! Location text → eligibility → verdict for one viewer, as a table. Each row
//! is a posting's location as a source writes it, whether the source says the
//! job is remote, and what someone in Indonesia (UTC+7) should be told.
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
        // Remote, with places: listed or inside a region is yes, else no.
        ("Indonesia; Malaysia", Some(true), Yes),
        ("Hong Kong; Singapore; Taiwan", Some(true), No),
        ("APAC", Some(true), Yes),
        ("Southeast Asia", Some(true), Yes),
        ("EMEA", Some(true), No),
        ("LATAM", Some(true), No),
        ("Europe; Indonesia", Some(true), Yes),
        ("Anywhere", Some(true), Yes),
        ("Remote", Some(true), Unknown),
        // Remote or on-site not stated (Greenhouse): tied to the places all the
        // same. Outside them, no; inside, maybe (it could be on-site).
        ("Hong Kong; Singapore; Taiwan", None, No),
        ("Indonesia; Malaysia; Philippines", None, Maybe),
        ("APAC; Hong Kong", None, Maybe),
        ("Berlin", None, No),
        ("Remote - Warsaw", Some(true), No),
        ("Jakarta", Some(true), Maybe),
        ("Remote, Colorado", Some(true), No),
        ("", None, Unknown),
        // On-site somewhere else.
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
