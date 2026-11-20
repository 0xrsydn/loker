//! Where a remote job may be done from, read from its description: "based in
//! Colombia, Mexico, or Argentina", "remote within the U.S.", "eligible to
//! work in Bulgaria", "CET (+/- 3 hours)". Used only to narrow what the
//! board's fields leave open (see [`narrow`]); `eval/eligibility` measures it.
//! A sentence about the company ("our team is based in Berlin", "customers in
//! the US") or one that excludes ("outside the US") is left out.
use std::sync::LazyLock;

use regex::Regex;

use crate::model::{Eligibility, Scope, TzStrength, TzWindow};

/// What a description says.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stated {
    pub countries: Vec<String>,
    pub regions: Vec<String>,
    /// Countries named only through a city: maybe an office there.
    pub only_cities: bool,
    pub worldwide: bool,
    pub tz: Option<TzWindow>,
    pub evidence: Vec<String>,
}

/// A phrase that says where the hire must be, followed by the place.
static WHERE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:(?:located|based)\s+in\s+and\s+(?:authori[sz]ed|eligible)\s+to\s+work\s+in|(?:based|located|living|residing|reside|resident|live)\s+(?:anywhere\s+)?(?:in|within)|(?:hiring|hire|recruiting)\s+(?:for\s+this\s+(?:role|position)\s+)?(?:across|in|within)|remote(?:ly)?(?:\s+based)?\s+(?:in|within|from)|fully\s+remote,?\s+(?:based\s+)?(?:in|within)|work(?:ing)?\s+(?:remotely\s+)?from(?:\s+anywhere\s+(?:in|within))?|anywhere\s+(?:in|within)|(?:eligible|authori[sz]ed|permitted|able)\s+to\s+(?:live\s+and\s+)?work\s+(?:remotely\s+)?in|open\s+to\s+(?:candidates|applicants|people|residents)\s+(?:based\s+)?(?:in|from))\s+([^.;!?()\n]{2,120})",
    )
    .unwrap()
});

/// Just before the phrase: a condition ("if you live in"), an exclusion, or
/// the company rather than the hire.
static NOT_THE_HIRE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:if|unless|not|outside|except|excluding|unable|cannot|can't|won't|customers?|clients?|users?|partners?|company|companies|headquarter\w*|hq|offices?|team|teams|startup|business(?:es)?)\b").unwrap()
});

/// Words before the place list.
static FILLER: LazyLock<[Regex; 3]> = LazyLock::new(|| {
    [
        Regex::new(r"(?i)^(?:either|any of|one of)\s+").unwrap(),
        Regex::new(r"(?i)^any\s+(?:city|cities|country|countries|state|states|location|locations)\s+(?:within|in)\s+").unwrap(),
        Regex::new(r"(?i)^(?:the\s+)?following\s+(?:US\s+)?(?:cities|countries|states|locations|regions)\s*:?\s*").unwrap(),
    ]
});

static OPEN_ENDED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bwherever\b|\banywhere\b").unwrap());

static STATES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bstates\b").unwrap());

static WORLDWIDE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bno\s+geographic(?:al)?\s+restrictions?\b|\b(?:work|hire|hiring)\s+(?:from\s+)?anywhere\s+in\s+the\s+world\b|\bopen\s+to\s+(?:candidates|applicants)\s+worldwide\b").unwrap()
});

/// "CET (+/- 3 hours)", "within 2 hours of EST", "UTC-3 to UTC+3".
static HOURS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)\b(?:(?P<z1>{Z})\s*(?:time\s*zone)?\s*\(?\s*(?:\+\s*/\s*-|±|\+-|plus\s+or\s+minus)\s*(?P<n1>\d{{1,2}}|{N})\s*(?:hours?|hrs?|h)\b|within\s+(?P<n2>\d{{1,2}}|{N})\s+hours?\s+of\s+(?P<z2>{Z})|between\s+(?P<a>{Z})\s+and\s+(?P<b>{Z})|(?P<c>UTC\s*[+-]\s*\d{{1,2}})\s*(?:to|-|–)\s*(?P<d>UTC\s*[+-]\s*\d{{1,2}}))",
        N = "one|two|three|four|five|six",
        Z = r"UTC\s*[+-]\s*\d{1,2}|GMT\s*[+-]\s*\d{1,2}|CEST|CET|EET|WET|GMT|UTC|BST|EST|EDT|ET|CST|CT|MST|MT|PST|PDT|PT|IST|SGT|AEST|AEDT|JST"
    ))
    .unwrap()
});

static PREFERRED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(?:prefer\w*|ideally|nice to have|bonus|plus)\b").unwrap());

fn zone(z: &str) -> Option<f64> {
    let z = z.to_uppercase().replace(' ', "");
    if let Some(rest) = z.strip_prefix("UTC").or_else(|| z.strip_prefix("GMT"))
        && !rest.is_empty()
    {
        return rest.parse().ok();
    }
    Some(match z.as_str() {
        "UTC" | "GMT" | "WET" => 0.0,
        "CET" | "BST" => 1.0,
        "CEST" | "EET" => 2.0,
        "IST" => 5.5,
        "SGT" => 8.0,
        "JST" => 9.0,
        "AEST" => 10.0,
        "AEDT" => 11.0,
        "EST" | "ET" => -5.0,
        "EDT" => -4.0,
        "CST" | "CT" => -6.0,
        "MST" | "MT" | "PDT" => -7.0,
        "PST" | "PT" => -8.0,
        _ => return None,
    })
}

fn hours(text: &str) -> Option<(TzWindow, String)> {
    let c = HOURS.captures(text)?;
    let g = |n: &str| c.name(n).map(|m| m.as_str());
    let (from, to) = if let (Some(z), Some(n)) = (g("z1").or(g("z2")), g("n1").or(g("n2"))) {
        let n = match n.to_lowercase().as_str() {
            "one" => 1.0,
            "two" => 2.0,
            "three" => 3.0,
            "four" => 4.0,
            "five" => 5.0,
            "six" => 6.0,
            d => d.parse().ok()?,
        };
        let o = zone(z)?;
        (o - n, o + n)
    } else {
        let (a, b) = (g("a").or(g("c"))?, g("b").or(g("d"))?);
        (zone(a)?, zone(b)?)
    };
    let all = c.get(0)?;
    let around = window(text, all.start().saturating_sub(80), all.end() + 80);
    let strength = if PREFERRED.is_match(around) {
        TzStrength::Preferred
    } else {
        TzStrength::Required
    };
    // A window that covers the whole day says nothing.
    (to - from < 23.0).then(|| (TzWindow { from, to, strength }, all.as_str().to_string()))
}

/// The leading places of a phrase ("Colombia, Mexico, or Argentina and
/// willing to travel" → CO, MX, AR), stopping at the first part that isn't one.
/// A two-letter code counts only in capitals: "US" is a place, "is" a word.
fn places(phrase: &str, out: &mut Stated) -> bool {
    // "one of the following states: Maine, ...", "any city within Italy",
    // "either the New York City or Boston metro areas".
    let had = !out.countries.is_empty() || !out.regions.is_empty();
    let rest = FILLER
        .iter()
        .fold(phrase, |p, rx| rx.find(p).map_or(p, |m| &p[m.end()..]));
    // "Europe, Singapore, or wherever you are": not a restriction.
    if OPEN_ENDED.is_match(rest) {
        return false;
    }
    // "any of the following states: AZ, CA, CO": US states, whatever their
    // codes spell as countries (CA, CO, IN, DE...).
    if STATES.is_match(&phrase[..phrase.len() - rest.len()]) {
        out.countries.push("US".into());
        out.only_cities = false;
        return true;
    }
    let phrase = rest;
    let mut found = false;
    let mut cities_only = true;
    for part in phrase
        .split([',', '/', '|'])
        .flat_map(|p| p.split(" or "))
        .flat_map(|p| p.split(" and "))
    {
        let p = part.trim();
        let p = ["or ", "and ", "the ", "The "]
            .iter()
            .fold(p, |p, w| p.strip_prefix(w).unwrap_or(p))
            .trim();
        if p.is_empty() {
            continue;
        }
        // "united states on a W2 basis": the longest leading words that name a place.
        let words: Vec<&str> = p.split_whitespace().collect();
        let hit = (1..=words.len().min(4)).rev().find_map(|n| {
            let name = words[..n].join(" ");
            if name.len() == 2 && name != name.to_uppercase() {
                return None;
            }
            crate::normalize::place(&name.to_lowercase())
        });
        match hit {
            Some(crate::normalize::Place::Region(r)) => {
                out.regions.push(r.to_string());
                cities_only = false;
            }
            Some(crate::normalize::Place::Country(c)) => {
                out.countries.push(c.to_string());
                cities_only = false;
            }
            Some(crate::normalize::Place::City(c)) => out.countries.push(c.to_string()),
            Some(crate::normalize::Place::Worldwide) | None => break,
        }
        found = true;
        if words.len() > 4 {
            break;
        }
    }
    // Only cities across every phrase read so far, not just this one.
    if found {
        out.only_cities = cities_only && (!had || out.only_cities);
    }
    found
}

fn window(s: &str, mut a: usize, b: usize) -> &str {
    let mut b = b.min(s.len());
    while !s.is_char_boundary(a) {
        a -= 1;
    }
    while !s.is_char_boundary(b) {
        b += 1;
    }
    &s[a..b]
}

/// Where the hire may be, and required hours, as `text` states them.
pub fn stated(text: &str) -> Option<Stated> {
    let text: String = text.chars().take(200_000).collect();
    let text = text
        .replace("\\", "")
        .replace(['*', '_'], "")
        .replace("U.S.A.", "USA")
        .replace("U.S.", "US")
        .replace("U.K.", "UK")
        .replace("E.U.", "EU");
    let mut out = Stated::default();
    for c in WHERE.captures_iter(&text) {
        let (all, phrase) = (c.get(0).unwrap(), c.get(1).unwrap());
        // The same clause, just before the phrase.
        let before = window(&text, all.start().saturating_sub(30), all.start());
        let clause = before
            .rsplit(['.', ';', ',', '!', '?', '\n'])
            .next()
            .unwrap_or(before);
        if NOT_THE_HIRE.is_match(clause) {
            continue;
        }
        if places(phrase.as_str(), &mut out) {
            out.evidence.push(all.as_str().trim().to_string());
        }
    }
    if out.countries.is_empty() && out.regions.is_empty() {
        out.only_cities = false;
        if let Some(m) = WORLDWIDE.find(&text) {
            out.worldwide = true;
            out.evidence.push(m.as_str().to_string());
        }
    }
    if let Some((w, ev)) = hours(&text) {
        out.tz = Some(w);
        out.evidence.push(ev);
    }
    out.countries.sort();
    out.countries.dedup();
    out.regions.sort();
    out.regions.dedup();
    (!out.evidence.is_empty()).then_some(out)
}

/// Narrow `e` with what the description says, only where the board's fields
/// leave it open, and never wider: places only when the board named none,
/// hours only when it gave none, nothing for on-site or hybrid jobs.
/// `remote` is whether the board says the job is remote (a remote-only
/// board, or its remote flag).
pub fn narrow(e: &mut Eligibility, description: &str, remote: bool) {
    if matches!(e.scope, Scope::Onsite | Scope::Hybrid) {
        return;
    }
    let Some(s) = stated(description) else {
        return;
    };
    let remote = remote
        || matches!(e.scope, Scope::Worldwide | Scope::Region | Scope::Countries)
        || e.raw
            .as_deref()
            .is_some_and(|r| r.to_lowercase().contains("remote"));
    let mut used = Vec::new();
    let open = e.countries.is_empty() && e.regions.is_empty();
    if open && (!s.countries.is_empty() || !s.regions.is_empty()) {
        e.countries = s.countries.clone();
        e.regions = s.regions.clone();
        // A remote job tied to places is open to them; a city alone may be
        // an office (maybe inside, no outside), as for an unknown scope.
        e.scope = match (remote && !s.only_cities, s.regions.is_empty()) {
            (false, _) => Scope::Unknown,
            (true, false) => Scope::Region,
            (true, true) => Scope::Countries,
        };
        e.confidence = 0.6;
        used.push("places");
    } else if open && s.worldwide && e.scope == Scope::Unknown && remote {
        e.scope = Scope::Worldwide;
        e.confidence = 0.5;
        used.push("worldwide");
    }
    if e.tz.is_none() && remote && s.tz.is_some() {
        e.tz = s.tz.clone();
        used.push("hours");
    }
    if !used.is_empty() {
        let ev = s.evidence.join(" … ");
        e.evidence = Some(format!(
            "description: {}",
            ev.chars().take(300).collect::<String>()
        ));
    }
}
