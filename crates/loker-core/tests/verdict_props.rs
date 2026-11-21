use isocountry::CountryCode;
use loker_core::country::region_contains;
use loker_core::eligibility_text::narrow;
use loker_core::model::{Eligibility, Elsewhere, Method, Scope, TzStrength, TzWindow, Workplace};
use loker_core::normalize::eligibility_from_text;
use loker_core::verdict::{VerdictKind, Viewer, eligible_for};
use proptest::prelude::*;

const REGIONS: &[&str] = &[
    "Europe",
    "EMEA",
    "Middle East",
    "Africa",
    "APAC",
    "Asia",
    "Southeast Asia",
    "Oceania",
    "North America",
    "South America",
    "LATAM",
    "Americas",
    "Atlantis",
];
const NAMES: &[&str] = &[
    "Germany",
    "united states",
    "USA",
    "UK",
    "Brazil",
    "India",
    "Indonesia",
    "Hong Kong",
    "Singapore",
    "Mexico",
    "Nigeria",
    "New Zealand",
    "Canada",
    "Japan",
    "EMEA",
    "APAC",
    "LATAM",
    "Europe",
    "Berlin",
    "Anywhere",
];

fn codes() -> Vec<&'static str> {
    CountryCode::iter().map(|c| c.alpha2()).collect()
}

fn half_hours(lo: f64, hi: f64) -> impl Strategy<Value = f64> {
    ((lo * 2.0) as i32..=(hi * 2.0) as i32).prop_map(|h| h as f64 / 2.0)
}

fn window() -> impl Strategy<Value = TzWindow> {
    (
        half_hours(-12.0, 11.5),
        half_hours(-12.0, 11.5),
        any::<bool>(),
    )
        .prop_map(|(from, to, req)| TzWindow {
            from,
            to,
            strength: if req {
                TzStrength::Required
            } else {
                TzStrength::Preferred
            },
        })
}

fn eligibility() -> impl Strategy<Value = Eligibility> {
    let workplace = prop::sample::select(vec![
        Workplace::Remote,
        Workplace::Hybrid,
        Workplace::Onsite,
        Workplace::Unknown,
    ]);
    let elsewhere = prop::sample::select(vec![
        Elsewhere::Excluded,
        Elsewhere::NotStated,
        Elsewhere::Open,
    ]);
    (
        workplace,
        elsewhere,
        prop::sample::subsequence(REGIONS.to_vec(), 0..3),
        prop::sample::subsequence(codes(), 0..6),
        prop::option::of(window()),
    )
        .prop_map(
            |(workplace, elsewhere, regions, countries, tz)| Eligibility {
                workplace,
                elsewhere,
                regions: regions.into_iter().map(String::from).collect(),
                countries: countries.into_iter().map(String::from).collect(),
                tz,
                confidence: 0.5,
                ..Eligibility::unknown(Method::Rules)
            },
        )
}

fn viewer() -> impl Strategy<Value = Viewer> {
    (
        prop::sample::select(codes()),
        prop::option::of(half_hours(-12.0, 14.0)),
    )
        .prop_map(|(c, tz)| Viewer {
            country: c.to_string(),
            tz,
            accept_onsite_in: Vec::new(),
        })
}

fn kind(e: &Eligibility, v: &Viewer) -> VerdictKind {
    eligible_for(e, v).kind
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn no_place_no_yes(mut e in eligibility(), v in viewer()) {
        if e.elsewhere == Elsewhere::Open {
            e.elsewhere = Elsewhere::NotStated;
        }
        e.countries.clear();
        e.regions.clear();
        let k = kind(&e, &v);
        prop_assert!(
            matches!(k, VerdictKind::Unknown | VerdictKind::No),
            "a job that names no place and doesn't say it is open anywhere can't tell anyone yes: {k:?}"
        );
    }

    #[test]
    fn listing_your_country_never_hurts(e in eligibility(), v in viewer()) {
        let mut listed = e.clone();
        listed.countries.push(v.country.clone());
        prop_assert!(
            kind(&listed, &v) <= kind(&e, &v),
            "listing the viewer's own country made their verdict worse (best to worst: yes, maybe, unknown, no)"
        );
    }

    #[test]
    fn region_verdict_follows_the_table(r in prop::sample::select(REGIONS.to_vec()), mut v in viewer()) {
        v.tz = None;
        let e = Eligibility {
            regions: vec![r.to_string()],
            ..unrestricted().with_scope(Scope::Region)
        };
        let want = match region_contains(r, &v.country) {
            Some(true) => VerdictKind::Yes,
            Some(false) => VerdictKind::No,
            None => VerdictKind::Unknown,
        };
        prop_assert_eq!(
            kind(&e, &v),
            want,
            "a remote job open to one region: yes inside it, no outside, unknown when the region isn't in the table"
        );
    }

    #[test]
    fn hours(e in eligibility(), w in window(), v in viewer()) {
        let base = kind(&Eligibility { tz: None, ..e.clone() }, &v);
        let with = kind(&Eligibility { tz: Some(w.clone()), ..e }, &v);
        let Some(off) = v.tz else {
            prop_assert_eq!(with, base);
            return Ok(());
        };
        match (w.contains(off), w.strength) {
            (true, _) => prop_assert_eq!(with, base, "hours the viewer can meet change nothing"),
            (false, TzStrength::Required) => prop_assert_eq!(with, VerdictKind::No, "required hours the viewer can't meet are a no"),
            (false, TzStrength::Preferred) => prop_assert_eq!(with, base.max(VerdictKind::Maybe), "preferred hours the viewer can't meet cap the verdict at maybe"),
        }
    }

    #[test]
    fn offsets_a_day_apart_are_the_same_hours(w in window(), off in half_hours(12.0, 14.0)) {
        prop_assert_eq!(w.contains(off), w.contains(off - 24.0), "UTC+13 and UTC-11 are the same clock time: {:?} at {}", w, off);
    }

    #[test]
    fn window_holds_its_offsets(offs in prop::collection::vec(half_hours(-12.0, 14.0), 1..8)) {
        if let Some(w) = TzWindow::from_offsets(&offs, TzStrength::Required) {
            for o in &offs {
                prop_assert!(w.contains(*o), "{:?} from {:?} leaves out {}", w, offs, o);
            }
        }
    }

    #[test]
    fn any_text_parses(text in ".{0,80}", remote in prop::option::of(any::<bool>())) {
        let e = eligibility_from_text(&text, remote);
        if remote == Some(false) {
            prop_assert_eq!(e.scope(), Scope::Onsite, "an explicit on-site flag is always on-site");
        }
    }

    #[test]
    fn description_never_overrides_the_board(
        e in eligibility(),
        sentence in prop::sample::select(vec![
            "Must be based in the United States.", "Remote within Europe.", "You will work remotely from Brazil.",
            "Located in CET timezone (+/- 3 hours).", "No geographical restrictions.", "Based in Berlin.",
        ]),
        remote in any::<bool>(),
    ) {
        let mut after = e.clone();
        narrow(&mut after, sentence, remote);
        if matches!(e.scope(), Scope::Onsite | Scope::Hybrid) {
            prop_assert_eq!(&after, &e, "on-site or hybrid jobs stay as they are");
        }
        if !e.countries.is_empty() || !e.regions.is_empty() {
            prop_assert_eq!((&after.scope(), &after.countries, &after.regions), (&e.scope(), &e.countries, &e.regions), "places the board named stay");
        }
        if e.tz.is_some() {
            prop_assert_eq!(&after.tz, &e.tz, "the board's hours stay");
        }
    }

    #[test]
    fn outside_follows_elsewhere(e in eligibility(), mut v in viewer()) {
        v.tz = None;
        let e = Eligibility { workplace: Workplace::Remote, tz: None, ..e };
        prop_assume!(region_known(&e) && e.names_places() && !inside(&e, &v.country));
        let want = match e.elsewhere {
            Elsewhere::Excluded => VerdictKind::No,
            Elsewhere::NotStated => VerdictKind::Unknown,
            Elsewhere::Open => VerdictKind::Maybe,
        };
        prop_assert_eq!(
            kind(&e, &v),
            want,
            "outside a remote job's places: excluded is no, not stated is unknown, open is maybe"
        );
    }

    #[test]
    fn wire_round_trip(e in eligibility()) {
        let json = serde_json::to_value(&e).unwrap();
        let back: Eligibility = serde_json::from_value(json.clone()).unwrap();
        prop_assert_eq!(&back, &e);
        prop_assert_eq!(&json["scope"], &serde_json::to_value(e.scope()).unwrap(), "its `scope` agrees");
    }

    #[test]
    fn order_and_case_dont_matter(names in prop::sample::subsequence(NAMES.to_vec(), 1..4), remote in prop::option::of(Just(true))) {
        let read = |t: &str| {
            let e = eligibility_from_text(t, remote);
            (e.scope(), e.regions, e.countries)
        };
        let forward = names.join("; ");
        let backward = names.iter().rev().cloned().collect::<Vec<_>>().join("; ");
        prop_assert_eq!(read(&forward), read(&backward));
        prop_assert_eq!(read(&forward), read(&forward.to_uppercase()));
    }
}

fn region_known(e: &Eligibility) -> bool {
    e.regions.iter().all(|r| region_contains(r, "US").is_some())
}

fn inside(e: &Eligibility, c: &str) -> bool {
    e.countries.iter().any(|x| x == c)
        || e.regions
            .iter()
            .any(|r| region_contains(r, c) == Some(true))
}

#[test]
fn old_rows_read_by_scope() {
    let old = |scope: &str| -> Eligibility {
        serde_json::from_value(serde_json::json!({
            "scope": scope, "countries": ["US"], "regions": [], "onsiteCity": null, "tz": null,
            "confidence": 0.7, "method": "rules", "evidence": null, "raw": null
        }))
        .unwrap()
    };
    assert_eq!(
        (old("countries").workplace, old("countries").elsewhere),
        (Workplace::Remote, Elsewhere::Excluded),
        "rows stored before the axes existed carry only `scope`, and read as it says"
    );
    assert_eq!(old("worldwide").elsewhere, Elsewhere::Open);
    assert_eq!(old("onsite").workplace, Workplace::Onsite);
    assert_eq!(old("unknown").workplace, Workplace::Unknown);
}

#[test]
fn parsed_regions_resolve() {
    for text in [
        "europe",
        "eu",
        "emea",
        "apac",
        "asia pacific",
        "asia",
        "southeast asia",
        "sea",
        "latam",
        "latin america",
        "americas",
        "north america",
        "south america",
        "africa",
        "middle east",
        "oceania",
    ] {
        for r in eligibility_from_text(text, Some(true)).regions {
            assert!(
                region_contains(&r, "US").is_some(),
                "the parser names a region the verdict can't resolve: {text} -> {r}"
            );
        }
    }
}

fn unrestricted() -> Eligibility {
    Eligibility {
        confidence: 0.5,
        ..Eligibility::unknown(Method::Rules)
    }
    .with_scope(Scope::Worldwide)
}
