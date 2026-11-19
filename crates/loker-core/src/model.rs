use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Career level, lowest first.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Seniority {
    Intern,
    Junior,
    Mid,
    Senior,
    /// Lead, staff or principal.
    Lead,
    /// Head of, director, VP, C-level.
    Executive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Worldwide,
    Region,
    Countries,
    Onsite,
    Hybrid,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TzStrength {
    Required,
    Preferred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    Source,
    Rules,
    Jev,
}

/// Allowed UTC offsets as a window in hours, e.g. `+1.0 ..= +7.0`.
/// The window may wrap around the date line (`from > to`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TzWindow {
    pub from: f64,
    pub to: f64,
    pub strength: TzStrength,
}

impl TzWindow {
    pub fn contains(&self, offset: f64) -> bool {
        if self.from <= self.to {
            (self.from..=self.to).contains(&offset)
        } else {
            offset >= self.from || offset <= self.to
        }
    }

    /// Build a window from a list of allowed offsets. Returns `None` when the list
    /// covers (nearly) every time zone, which means "no restriction".
    pub fn from_offsets(offsets: &[f64], strength: TzStrength) -> Option<Self> {
        let mut xs: Vec<f64> = offsets.iter().map(|o| normalize_offset(*o)).collect();
        xs.sort_by(|a, b| a.total_cmp(b));
        xs.dedup();
        if xs.is_empty() {
            return None;
        }
        // The window is the complement of the largest gap between neighbours on the 24h circle.
        let n = xs.len();
        let (mut gap, mut after) = (xs[0] + 24.0 - xs[n - 1], 0);
        for i in 1..n {
            let g = xs[i] - xs[i - 1];
            if g > gap {
                gap = g;
                after = i;
            }
        }
        if gap <= 2.0 {
            return None;
        }
        Some(TzWindow {
            from: xs[after],
            to: xs[(after + n - 1) % n],
            strength,
        })
    }
}

/// Map an offset into `[-12, 12)`, so `+14` (Kiribati) sits next to `-10`.
fn normalize_offset(o: f64) -> f64 {
    let mut x = o;
    while x >= 12.0 {
        x -= 24.0;
    }
    while x < -12.0 {
        x += 24.0;
    }
    x
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Eligibility {
    pub scope: Scope,
    #[serde(default)]
    pub regions: Vec<String>,
    /// ISO 3166-1 alpha-2 codes.
    #[serde(default)]
    pub countries: Vec<String>,
    pub onsite_city: Option<String>,
    pub tz: Option<TzWindow>,
    pub confidence: f64,
    pub method: Method,
    pub evidence: Option<String>,
    /// Original location text, kept for the evidence trail.
    pub raw: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Pay {
    pub usd_month_min: Option<u32>,
    pub usd_month_max: Option<u32>,
    pub raw: String,
}

/// Employer boards (Greenhouse, Ashby, …) are the primary record of a job;
/// aggregators repost them. When both have a job, the employer's fields win.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Employer,
    Aggregator,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Employer => "employer",
            SourceKind::Aggregator => "aggregator",
        }
    }
}

/// A job as an adapter produces it, normalized, before it is linked to a
/// canonical job. Stored per source (without `raw`) so the canonical job can be
/// rebuilt from all its sources.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewJob {
    pub source_id: String,
    pub source_job_id: String,
    pub source_url: String,
    pub canonical_url: String,
    pub apply_url: Option<String>,
    pub title: String,
    pub company: String,
    pub seniority: Option<String>,
    pub employment_type: Option<String>,
    pub eligibility: Eligibility,
    pub pay: Option<Pay>,
    pub tech_tags: Vec<String>,
    pub description_md: Option<String>,
    pub posted_at: Option<DateTime<Utc>>,
    /// When the source says the listing ends, if it says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(skip)]
    pub raw: serde_json::Value,
}

/// A stored job, as functions return it.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub title: String,
    pub company: String,
    /// From the title when it names a level, else as the source states it;
    /// absent when neither does.
    pub seniority: Option<Seniority>,
    pub employment_type: Option<String>,
    pub eligibility: Eligibility,
    pub pay: Option<Pay>,
    pub tech_tags: Vec<String>,
    pub url: String,
    pub posted_at: Option<DateTime<Utc>>,
    pub first_seen: DateTime<Utc>,
    pub status: String,
    /// Every source that lists this job, with its page there (for attribution).
    pub sources: Vec<JobSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobSource {
    pub source: String,
    pub url: String,
}

#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IngestStats {
    pub fetched: u32,
    /// Source jobs seen for the first time.
    pub new: u32,
    /// Source jobs whose normalized content changed.
    pub updated: u32,
    pub unchanged: u32,
    /// New or updated source jobs that share a canonical job with another source.
    pub merged: u32,
    /// New or updated source jobs whose company + title key is shared by more
    /// than one job from the same source, so no cross-source merge was made.
    pub ambiguous: u32,
    /// Listings closed during this run (any source: absent, expired or stale).
    pub closed: u32,
    /// Gone listings seen again.
    pub reopened: u32,
    /// Listings missing from this fetch but not closed, because the fetch
    /// looked truncated (it would have closed more than half the source).
    pub held: u32,
    pub skipped: u32,
}
