use loker_core::eligibility_text::stated;

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
        (
            "Candidates must be located in and authorized to work in the United States.",
            &["US"],
            None,
        ),
        (
            "This role is open for candidates residing in the following US states: Maine, Vermont, Ohio.",
            &["US"],
            None,
        ),
        (
            "You are based in any city within Italy or Spain, with a valid right to work.",
            &["ES", "IT"],
            None,
        ),
        (
            "We're currently hiring for this role in Canada or the United States.",
            &["CA", "US"],
            None,
        ),
        (
            "Remote, with required availability within CET plus or minus three hours.",
            &[],
            Some((-2.0, 4.0)),
        ),
        ("**Location:** Remote – LATAM", &["LATAM"], None),
        (
            "Sourcing Location : South Africa/Philippines",
            &["PH", "ZA"],
            None,
        ),
        (
            "Location: Bangladesh, India, Pakistan, Indonesia",
            &["BD", "ID", "IN", "PK"],
            None,
        ),
        ("Office location: Berlin", &[], None),
        ("Location: San Francisco or Singapore", &[], None),
        ("Location: Remote", &[], None),
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
fn a_later_city_keeps_the_country() {
    let s =
        stated("Must be based in the United States. Based in New York, you will lead the team.")
            .unwrap();
    assert!(
        !s.only_cities,
        "a city read after a country doesn't make the job only a city: {s:?}"
    );
    assert!(
        stated("Based in Berlin, you will lead the team.")
            .unwrap()
            .only_cities
    );
}

#[test]
fn no_geographical_restrictions_is_worldwide() {
    assert!(
        stated("Fully remote position with no geographical restrictions.")
            .unwrap()
            .worldwide
    );
}
