//! The per-viewer eligibility verdict: computed per request, never stored.
use schemars::JsonSchema;
use serde::Serialize;

use crate::model::{Eligibility, Scope, TzStrength};

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

/// Whether `country` is among the job's countries or inside one of its
/// regions; `None` when it isn't listed and a region can't be resolved.
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

/// "APAC, HK, SG", for reasons.
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
    let (mut kind, mut reason) = match e.scope {
        Scope::Worldwide => (Yes, "worldwide".to_string()),
        Scope::Countries if e.countries.iter().any(|c| c == &v.country) => {
            (Yes, format!("{} listed", v.country))
        }
        Scope::Countries => (
            No,
            format!(
                "{} not among {} allowed countries",
                v.country,
                e.countries.len()
            ),
        ),
        Scope::Region => match within(e, &v.country) {
            Some(true) => (Yes, format!("{} in {}", v.country, places(e))),
            Some(false) => (No, format!("{} not in {}", v.country, places(e))),
            None => (
                Unknown,
                format!("region {} not resolved", e.regions.join("/")),
            ),
        },
        Scope::Onsite | Scope::Hybrid => match &e.onsite_city {
            Some(city)
                if v.accept_onsite_in
                    .iter()
                    .any(|c| c.eq_ignore_ascii_case(city)) =>
            {
                (Yes, format!("{city}, accepted onsite"))
            }
            Some(city) => (No, format!("onsite in {city}")),
            None => (No, "onsite".to_string()),
        },
        // Remote or on-site unknown, but tied to these places: outside them
        // it's no either way; inside, it may still be on-site.
        Scope::Unknown if !e.countries.is_empty() || !e.regions.is_empty() => {
            match within(e, &v.country) {
                Some(true) => (
                    Maybe,
                    format!(
                        "{} in {}, but remote or on-site isn't stated",
                        v.country,
                        places(e)
                    ),
                ),
                Some(false) => (No, format!("only {}", places(e))),
                None => (Unknown, format!("doesn't say if {} is ok", v.country)),
            }
        }
        Scope::Unknown => (Unknown, format!("doesn't say if {} is ok", v.country)),
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
