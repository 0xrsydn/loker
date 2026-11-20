//! Rules the verdict must keep for any eligibility and any viewer, checked on
//! generated inputs (`tests/eligibility.rs` holds the worked examples). The
//! verdicts order from best to worst: yes, maybe, unknown, no.
use isocountry::CountryCode;
use loker_core::country::region_contains;
use loker_core::model::{Eligibility, Method, Scope, TzStrength, TzWindow};
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
    let scope = prop::sample::select(vec![
        Scope::Worldwide,
        Scope::Region,
        Scope::Countries,
        Scope::Onsite,
        Scope::Hybrid,
        Scope::Unknown,
    ]);
    (
        scope,
        prop::sample::subsequence(REGIONS.to_vec(), 0..3),
        prop::sample::subsequence(codes(), 0..6),
        prop::option::of(window()),
    )
        .prop_map(|(scope, regions, countries, tz)| Eligibility {
            scope,
            regions: regions.into_iter().map(String::from).collect(),
            countries: countries.into_iter().map(String::from).collect(),
            onsite_city: None,
            tz,
            confidence: 0.5,
            method: Method::Rules,
            evidence: None,
            raw: None,
        })
}

/// Anyone on Earth: UTC-12 to UTC+14 (Kiribati).
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

    /// A job that names no place and isn't worldwide can't tell anyone yes.
    #[test]
    fn no_place_no_yes(mut e in eligibility(), v in viewer()) {
        e.scope = Scope::Unknown;
        e.countries.clear();
        e.regions.clear();
        let k = kind(&e, &v);
        prop_assert!(matches!(k, VerdictKind::Unknown | VerdictKind::No), "{k:?}");
    }

    /// Listing the viewer's own country never makes their verdict worse.
    #[test]
    fn listing_your_country_never_hurts(e in eligibility(), v in viewer()) {
        let mut listed = e.clone();
        listed.countries.push(v.country.clone());
        prop_assert!(kind(&listed, &v) <= kind(&e, &v));
    }

    /// A remote job open to one region: yes inside it, no outside, unknown
    /// when the region isn't in the table, exactly as the table says.
    #[test]
    fn region_verdict_follows_the_table(r in prop::sample::select(REGIONS.to_vec()), mut v in viewer()) {
        v.tz = None;
        let e = Eligibility { scope: Scope::Region, regions: vec![r.to_string()], ..unrestricted() };
        let want = match region_contains(r, &v.country) {
            Some(true) => VerdictKind::Yes,
            Some(false) => VerdictKind::No,
            None => VerdictKind::Unknown,
        };
        prop_assert_eq!(kind(&e, &v), want);
    }

    /// Required hours the viewer can't meet are a no; hours they can meet
    /// change nothing; preferred hours they can't meet cap the verdict at maybe.
    #[test]
    fn hours(e in eligibility(), w in window(), v in viewer()) {
        let base = kind(&Eligibility { tz: None, ..e.clone() }, &v);
        let with = kind(&Eligibility { tz: Some(w.clone()), ..e }, &v);
        let Some(off) = v.tz else {
            prop_assert_eq!(with, base);
            return Ok(());
        };
        match (w.contains(off), w.strength) {
            (true, _) => prop_assert_eq!(with, base),
            (false, TzStrength::Required) => prop_assert_eq!(with, VerdictKind::No),
            (false, TzStrength::Preferred) => prop_assert_eq!(with, base.max(VerdictKind::Maybe)),
        }
    }

    /// UTC+13 and UTC-11 are the same clock time: a window can't tell them apart.
    #[test]
    fn offsets_a_day_apart_are_the_same_hours(w in window(), off in half_hours(12.0, 14.0)) {
        prop_assert_eq!(w.contains(off), w.contains(off - 24.0), "{:?} at {}", w, off);
    }

    /// A window built from allowed offsets lets every one of them in.
    #[test]
    fn window_holds_its_offsets(offs in prop::collection::vec(half_hours(-12.0, 14.0), 1..8)) {
        if let Some(w) = TzWindow::from_offsets(&offs, TzStrength::Required) {
            for o in &offs {
                prop_assert!(w.contains(*o), "{:?} from {:?} leaves out {}", w, offs, o);
            }
        }
    }

    /// Any text parses, and an explicit on-site flag is always on-site.
    #[test]
    fn any_text_parses(text in ".{0,80}", remote in prop::option::of(any::<bool>())) {
        let e = eligibility_from_text(&text, remote);
        if remote == Some(false) {
            prop_assert_eq!(e.scope, Scope::Onsite);
        }
    }

    /// The order and case places are written in don't change what is read.
    #[test]
    fn order_and_case_dont_matter(names in prop::sample::subsequence(NAMES.to_vec(), 1..4), remote in prop::option::of(Just(true))) {
        let read = |t: &str| {
            let e = eligibility_from_text(t, remote);
            (e.scope, e.regions, e.countries)
        };
        let forward = names.join("; ");
        let backward = names.iter().rev().cloned().collect::<Vec<_>>().join("; ");
        prop_assert_eq!(read(&forward), read(&backward));
        prop_assert_eq!(read(&forward), read(&forward.to_uppercase()));
    }
}

/// Every region the parser can name is one the verdict can resolve.
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
            assert!(region_contains(&r, "US").is_some(), "{text} -> {r}");
        }
    }
}

fn unrestricted() -> Eligibility {
    Eligibility {
        scope: Scope::Worldwide,
        regions: Vec::new(),
        countries: Vec::new(),
        onsite_city: None,
        tz: None,
        confidence: 0.5,
        method: Method::Rules,
        evidence: None,
        raw: None,
    }
}
