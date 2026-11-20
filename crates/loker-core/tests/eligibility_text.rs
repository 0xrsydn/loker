//! Where a remote job may be done from, as its description says.
use loker_core::eligibility_text::stated;

/// Text, the places read (regions first), the hours window read.
type Case = (&'static str, &'static [&'static str], Option<(f64, f64)>);

#[test]
fn places_and_hours_read_from_descriptions() {
    let cases: &[Case] = &[
        (
            "Location: Fully remote, based in Colombia, Mexico, or Argentina.",
            &["AR", "CO", "MX"],
            None,
        ),
        ("Fully remote within the U.S.", &["US"], None),
        ("You will work remotely from Bulgaria.", &["BG"], None),
        (
            "Must be eligible to live and work in the United States.",
            &["US"],
            None,
        ),
        ("This position is remotely based in Kansas.", &["US"], None),
        ("Remote role based in Latin America.", &["LATAM"], None),
        (
            "Located in CET timezone (+/- 3 hours), we can't consider other time zones.",
            &[],
            Some((-2.0, 4.0)),
        ),
        (
            "Work hours: within 2 hours of EST.",
            &[],
            Some((-7.0, -3.0)),
        ),
        // Not where the hire must be.
        ("Our team is based in Berlin.", &[], None),
        ("We serve customers based in the US.", &[], None),
        (
            "This role must be performed outside the United States.",
            &[],
            None,
        ),
        (
            "If you live in the Bay Area, you're welcome to use our SF office.",
            &[],
            None,
        ),
        ("Ideal for people who live in is on the list.", &[], None),
    ];
    for &(text, places, hours) in cases {
        let s = stated(text);
        let got: Vec<String> = s
            .iter()
            .flat_map(|s| s.regions.iter().chain(&s.countries).cloned())
            .collect();
        assert_eq!(got, places, "{text}");
        let tz = s.and_then(|s| s.tz).map(|w| (w.from, w.to));
        assert_eq!(tz, hours, "{text}");
    }
}

#[test]
fn no_geographical_restrictions_is_worldwide() {
    assert!(
        stated("Fully remote position with no geographical restrictions.")
            .unwrap()
            .worldwide
    );
}
