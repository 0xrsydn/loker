use std::sync::LazyLock;

use regex::Regex;
use regex_automata::meta::Regex as MetaRegex;
use regex_automata::util::captures::Captures;
use regex_automata::{Anchored, Input};

use crate::model::{Eligibility, Elsewhere, TzStrength, TzWindow, Workplace};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stated {
    pub countries: Vec<String>,
    pub regions: Vec<String>,
    pub only_cities: bool,
    pub worldwide: bool,
    pub tz: Option<TzWindow>,
    pub evidence: Vec<String>,
}

struct Gated {
    gate: Regex,
    real: MetaRegex,
}

#[derive(Clone, Copy)]
struct Found<'t> {
    text: &'t str,
    start: usize,
    end: usize,
}

impl<'t> Found<'t> {
    fn start(&self) -> usize {
        self.start
    }
    fn end(&self) -> usize {
        self.end
    }
    fn as_str(&self) -> &'t str {
        &self.text[self.start..self.end]
    }
}

struct Hit<'t> {
    text: &'t str,
    caps: Captures,
}

impl<'t> Hit<'t> {
    fn found(&self, span: Option<regex_automata::Span>) -> Option<Found<'t>> {
        span.map(|s| Found {
            text: self.text,
            start: s.start,
            end: s.end,
        })
    }
    fn get(&self, i: usize) -> Option<Found<'t>> {
        self.found(self.caps.get_group(i))
    }
    fn name(&self, n: &str) -> Option<Found<'t>> {
        self.found(self.caps.get_group_by_name(n))
    }
}

struct Hits<'t> {
    re: &'t Gated,
    text: &'t str,
    pos: usize,
}

impl Hits<'_> {
    fn skip_to(&mut self, pos: usize) {
        self.pos = pos;
        while !self.text.is_char_boundary(self.pos) {
            self.pos += 1;
        }
    }
}

impl<'t> Iterator for Hits<'t> {
    type Item = Hit<'t>;
    fn next(&mut self) -> Option<Hit<'t>> {
        while self.pos <= self.text.len() {
            let candidate = self.re.gate.find_at(self.text, self.pos)?.start();
            let input = Input::new(self.text)
                .span(candidate..self.text.len())
                .anchored(Anchored::Yes);
            let mut caps = self.re.real.create_captures();
            self.re.real.search_captures(&input, &mut caps);
            if let Some(m) = caps.get_match() {
                self.skip_to(m.end().max(candidate + 1));
                return Some(Hit {
                    text: self.text,
                    caps,
                });
            }
            self.skip_to(candidate + 1);
        }
        None
    }
}

impl Gated {
    fn new(pattern: &str) -> Gated {
        Gated {
            gate: Regex::new(&pattern.replace(r"\b", "")).unwrap(),
            real: MetaRegex::new(pattern).unwrap(),
        }
    }

    fn captures_iter<'t>(&'t self, text: &'t str) -> Hits<'t> {
        Hits {
            re: self,
            text,
            pos: 0,
        }
    }

    fn is_match(&self, text: &str) -> bool {
        self.captures_iter(text).next().is_some()
    }
}

static WHERE: LazyLock<Gated> = LazyLock::new(|| {
    Gated::new(
        r"(?i)\b(?:location\s*:\s*(?:(?:fully\s+)?remote\s*(?:[-–—,(]|\bin\b|\bfrom\b)?)?|(?:located|based)\s+in\s+and\s+(?:authori[sz]ed|eligible)\s+to\s+work\s+in|(?:based|located|living|residing|reside|resident|live)\s+(?:anywhere\s+)?(?:in|within)|(?:hiring|hire|recruiting)\s+(?:for\s+this\s+(?:role|position)\s+)?(?:across|in|within)|remote(?:ly)?(?:\s+based)?\s+(?:in|within|from)|fully\s+remote,?\s+(?:based\s+)?(?:in|within)|work(?:ing)?\s+(?:remotely\s+)?from(?:\s+anywhere\s+(?:in|within))?|anywhere\s+(?:in|within)|(?:eligible|authori[sz]ed|permitted|able)\s+to\s+(?:live\s+and\s+)?work\s+(?:remotely\s+)?in|open\s+to\s+(?:candidates|applicants|people|residents)\s+(?:based\s+)?(?:in|from))\s+([^.;!?()\n]{2,120})",
    )
});

static NOT_THE_HIRE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:if|unless|not|outside|except|excluding|unable|cannot|can't|won't|customers?|clients?|users?|partners?|company|companies|headquarter\w*|hq|offices?|team|teams|startup|business(?:es)?)\b").unwrap()
});

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

static WORLDWIDE: LazyLock<Gated> = LazyLock::new(|| {
    Gated::new(
        r"(?i)(?:\bno\s+geographic(?:al)?\s+restrictions?\b|\b(?:work(?:ing)?|hire|hiring)\s+(?:from\s+)?anywhere\s+in\s+the\s+world\b|\bopen\s+to\s+(?:candidates|applicants)\s+worldwide\b)(?P<next>\s+\w+)?",
    )
});

static HOURS: LazyLock<Gated> = LazyLock::new(|| {
    Gated::new(&format!(
        r"(?i)\b(?:(?P<z1>{Z})\s*(?:time\s*zone)?\s*\(?\s*(?:\+\s*/\s*-|±|\+-|plus\s+or\s+minus)\s*(?P<n1>\d{{1,2}}|{N})\s*(?:hours?|hrs?|h)\b|within\s+(?P<n2>\d{{1,2}}|{N})\s+hours?\s+of\s+(?P<z2>{Z})|between\s+(?P<a>{Z})\s+and\s+(?P<b>{Z})|(?P<c>UTC\s*[+-]\s*\d{{1,2}})\s*(?:to|-|–)\s*(?P<d>UTC\s*[+-]\s*\d{{1,2}}))",
        N = "one|two|three|four|five|six",
        Z = r"UTC\s*[+-]\s*\d{1,2}|GMT\s*[+-]\s*\d{1,2}|CEST|CET|EET|WET|GMT|UTC|BST|EST|EDT|ET|CST|CT|MST|MT|PST|PDT|PT|IST|SGT|AEST|AEDT|JST"
    ))
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
    let c = HOURS.captures_iter(text).next()?;
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

fn names_a_city(phrase: &str) -> bool {
    phrase
        .split([',', '/', '|', ';'])
        .flat_map(|p| p.split(" or "))
        .flat_map(|p| p.split(" and "))
        .any(|p| {
            matches!(
                crate::normalize::place(&p.trim().to_lowercase()),
                Some(crate::normalize::Place::City(_))
            )
        })
}

fn places(phrase: &str, out: &mut Stated) -> bool {
    let had = !out.countries.is_empty() || !out.regions.is_empty();
    let rest = FILLER
        .iter()
        .fold(phrase, |p, rx| rx.find(p).map_or(p, |m| &p[m.end()..]));
    if OPEN_ENDED.is_match(rest) {
        return false;
    }
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

pub fn open_wider(text: &str) -> bool {
    static OPEN: LazyLock<Gated> = LazyLock::new(|| {
        Gated::new(
            r"(?i)\bno\s+geographic(?:al)?\s+restrictions?\b|\banywhere\s+in\s+the\s+world\b|\bremote\s+worldwide\b|\b(?:hire|hiring)\s+(?:worldwide|globally|internationally)\b|\blive\s+anywhere\b|\bfrom\s+wherever\s+you\s+are\b|\bwherever\s+you\s+(?:like|want|choose|prefer)\b|\blocation:\s*international\b|\bany\s+time\s*zones?\b|\bbased\s+outside\b",
        )
    });
    static ANYWHERE: LazyLock<Gated> = LazyLock::new(|| {
        Gated::new(
            r"(?i)\b(?:work|working)\s+(?:flexibly\s+|remotely\s+)?from\s+anywhere\b(?P<next>\s+\w+)?",
        )
    });
    let text = text.replace(['*', '_'], "");
    OPEN.is_match(&text)
        || ANYWHERE.captures_iter(&text).any(|c| {
            !c.name("next").is_some_and(|n| {
                matches!(n.as_str().trim().to_lowercase().as_str(), "in" | "within")
            })
        })
}

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
        let before = window(&text, all.start().saturating_sub(30), all.start());
        let clause = before
            .rsplit(['.', ';', ',', '!', '?', '\n'])
            .next()
            .unwrap_or(before);
        if NOT_THE_HIRE.is_match(clause) {
            continue;
        }
        let line = all.as_str().trim_start().to_lowercase();
        if line.starts_with("location") && !line.contains("remote") && names_a_city(phrase.as_str())
        {
            continue;
        }
        if places(phrase.as_str(), &mut out) {
            out.evidence.push(all.as_str().trim().to_string());
        } else {
            for inner in WHERE.captures_iter(phrase.as_str()) {
                if places(inner.get(1).unwrap().as_str(), &mut out) {
                    out.evidence
                        .push(inner.get(0).unwrap().as_str().trim().to_string());
                }
            }
        }
    }
    if out.countries.is_empty() && out.regions.is_empty() {
        out.only_cities = false;
        let open = WORLDWIDE.captures_iter(&text).find(|c| {
            !c.name("next").is_some_and(|n| {
                matches!(n.as_str().trim().to_lowercase().as_str(), "in" | "within")
            })
        });
        if let Some(c) = open {
            out.worldwide = true;
            out.evidence
                .push(c.get(0).unwrap().as_str().trim().to_string());
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

pub fn narrow(e: &mut Eligibility, description: &str, remote: bool) {
    if matches!(e.workplace, Workplace::Onsite | Workplace::Hybrid) {
        return;
    }
    let Some(s) = stated(description) else {
        return;
    };
    let remote = remote
        || e.workplace == Workplace::Remote
        || e.raw
            .as_deref()
            .is_some_and(|r| r.to_lowercase().contains("remote"));
    let mut used = Vec::new();
    let open = !e.names_places();
    if open && (!s.countries.is_empty() || !s.regions.is_empty()) {
        e.countries = s.countries.clone();
        e.regions = s.regions.clone();
        e.workplace = if remote && !s.only_cities {
            Workplace::Remote
        } else {
            Workplace::Unknown
        };
        e.elsewhere = Elsewhere::Excluded;
        e.confidence = 0.6;
        used.push("places");
    } else if open && s.worldwide && remote {
        e.workplace = Workplace::Remote;
        e.elsewhere = Elsewhere::Open;
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
