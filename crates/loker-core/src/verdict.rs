//! The per-viewer eligibility verdict: computed per request, never stored.
use serde::Serialize;

use crate::model::{Eligibility, Scope, TzStrength};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictKind {
    Yes,
    Maybe,
    Unknown,
    No,
}

#[derive(Debug, Clone, Serialize)]
pub struct Verdict {
    pub kind: VerdictKind,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct Viewer {
    /// ISO 3166-1 alpha-2.
    pub country: String,
    /// UTC offset in hours.
    pub tz: Option<f32>,
    pub accept_onsite_in: Vec<String>,
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
        Scope::Region => (
            Unknown,
            format!("region {} not resolved yet", e.regions.join("/")),
        ),
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
