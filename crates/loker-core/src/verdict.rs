use schemars::JsonSchema;
use serde::Serialize;

use crate::model::{Eligibility, Elsewhere, TzStrength, Workplace};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VerdictKind {
    Yes,
    Maybe,
    Unknown,
    No,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Verdict {
    pub kind: VerdictKind,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct Viewer {
    /// ISO 3166-1 alpha-2.
    pub country: String,
    /// UTC offset in hours.
    pub tz: Option<f64>,
    pub accept_onsite_in: Vec<String>,
}

fn within(e: &Eligibility, country: &str) -> Option<bool> {
    if e.countries.iter().any(|c| c == country) {
        return Some(true);
    }
    let mut known = true;
    for r in &e.regions {
        match crate::country::region_contains(r, country) {
            Some(true) => return Some(true),
            Some(false) => {}
            None => known = false,
        }
    }
    known.then_some(false)
}

fn places(e: &Eligibility) -> String {
    let all: Vec<&str> = e
        .regions
        .iter()
        .chain(&e.countries)
        .map(String::as_str)
        .collect();
    match all.len() {
        0..=4 => all.join(", "),
        n => format!("{} and {} more", all[..3].join(", "), n - 3),
    }
}

pub fn eligible_for(e: &Eligibility, v: &Viewer) -> Verdict {
    use VerdictKind::*;
    let c = &v.country;
    let outside = |places: String| match e.elsewhere {
        Elsewhere::Excluded => (No, format!("{c} not in {places}")),
        Elsewhere::NotStated => (Unknown, format!("lists {places}; doesn't say if {c} is ok")),
        Elsewhere::Open => (
            Maybe,
            format!("lists {places}, and says it is open more widely"),
        ),
    };
    let (mut kind, mut reason) = match e.workplace {
        Workplace::Onsite | Workplace::Hybrid => {
            let what = if e.workplace == Workplace::Hybrid {
                "hybrid"
            } else {
                "onsite"
            };
            match &e.onsite_city {
                Some(city)
                    if v.accept_onsite_in
                        .iter()
                        .any(|a| a.eq_ignore_ascii_case(city)) =>
                {
                    (Yes, format!("{city}, accepted onsite"))
                }
                Some(city) => (No, format!("{what} in {city}")),
                None => (No, what.to_string()),
            }
        }
        Workplace::Remote if !e.names_places() => match e.elsewhere {
            Elsewhere::Open => (Yes, "worldwide".to_string()),
            _ => (Unknown, format!("remote, but doesn't say if {c} is ok")),
        },
        Workplace::Remote => match within(e, c) {
            Some(true) => (Yes, format!("{c} in {}", places(e))),
            Some(false) => outside(places(e)),
            None => (
                Unknown,
                format!("region {} not resolved", e.regions.join("/")),
            ),
        },
        Workplace::Unknown if !e.names_places() => (Unknown, format!("doesn't say if {c} is ok")),
        Workplace::Unknown => match within(e, c) {
            Some(true) => (
                Maybe,
                format!("{c} in {}, but remote or on-site isn't stated", places(e)),
            ),
            Some(false) => outside(places(e)),
            None => (Unknown, format!("doesn't say if {c} is ok")),
        },
    };
    if let (Some(w), Some(off)) = (&e.tz, v.tz) {
        let window = format!("UTC{:+}..{:+}", w.from, w.to);
        match (w.contains(off), w.strength) {
            (true, _) => reason.push_str(&format!(", inside {window}")),
            (false, TzStrength::Required) => {
                kind = No;
                reason = format!("requires {window}");
            }
            (false, TzStrength::Preferred) => {
                kind = kind.max(Maybe);
                reason.push_str(&format!(", outside preferred {window}"));
            }
        }
    }
    Verdict { kind, reason }
}
