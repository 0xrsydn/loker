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

/// The eligibility summary every contract version has carried, derived from
/// `workplace`, the places and `elsewhere` (see [`Eligibility::scope`]).
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

/// Whether the job is done remotely, as the posting or board says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Workplace {
    Remote,
    Hybrid,
    Onsite,
    /// Not stated: a city or country alone may be an office.
    Unknown,
}

/// What the posting says about people outside its places. A list of places
/// is not always a limit: "USA Only" is, an employer's office on a remote job
/// isn't. Gray information gets its own value instead of a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Elsewhere {
    /// Only the listed places ("USA Only", "must reside in Germany").
    Excluded,
    /// Places are listed, nothing is said about anyone else.
    NotStated,
    /// Open beyond them ("worldwide", "or wherever you are").
    Open,
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
    /// Offsets a day apart are the same hours: UTC+13 (New Zealand in summer)
    /// is inside a window that lists it as UTC-11.
    pub fn contains(&self, offset: f64) -> bool {
        let (from, to, offset) = (
            normalize_offset(self.from),
            normalize_offset(self.to),
            normalize_offset(offset),
        );
        if from <= to {
            (from..=to).contains(&offset)
        } else {
            offset >= from || offset <= to
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

/// Who can take a job, as three answers: is it remote (`workplace`), where
/// it is open (`regions`, `countries`; for an office job, where the office
/// is), and what it says about everyone else (`elsewhere`). `scope` on the
/// wire is derived from them, for clients of earlier contract versions.
#[derive(Debug, Clone, PartialEq)]
pub struct Eligibility {
    pub workplace: Workplace,
    pub elsewhere: Elsewhere,
    pub regions: Vec<String>,
    /// ISO 3166-1 alpha-2 codes.
    pub countries: Vec<String>,
    pub onsite_city: Option<String>,
    pub tz: Option<TzWindow>,
    pub confidence: f64,
    pub method: Method,
    pub evidence: Option<String>,
    /// Original location text, kept for the evidence trail.
    pub raw: Option<String>,
}

impl Eligibility {
    /// Nothing known yet: no places, remote or not unstated.
    pub fn unknown(method: Method) -> Self {
        Eligibility {
            workplace: Workplace::Unknown,
            elsewhere: Elsewhere::Excluded,
            regions: Vec::new(),
            countries: Vec::new(),
            onsite_city: None,
            tz: None,
            confidence: 0.0,
            method,
            evidence: None,
            raw: None,
        }
    }

    /// The axes a `scope` stands for: how rows stored before they existed
    /// read, and a shorthand for building one.
    pub fn axes_of(scope: Scope) -> (Workplace, Elsewhere) {
        match scope {
            Scope::Worldwide => (Workplace::Remote, Elsewhere::Open),
            Scope::Region | Scope::Countries => (Workplace::Remote, Elsewhere::Excluded),
            Scope::Onsite => (Workplace::Onsite, Elsewhere::Excluded),
            Scope::Hybrid => (Workplace::Hybrid, Elsewhere::Excluded),
            Scope::Unknown => (Workplace::Unknown, Elsewhere::Excluded),
        }
    }

    /// Set both axes from a `scope`.
    pub fn with_scope(mut self, scope: Scope) -> Self {
        (self.workplace, self.elsewhere) = Self::axes_of(scope);
        self
    }

    pub fn names_places(&self) -> bool {
        !self.countries.is_empty() || !self.regions.is_empty()
    }

    /// The one-word summary earlier contract versions carried.
    pub fn scope(&self) -> Scope {
        match self.workplace {
            Workplace::Onsite => Scope::Onsite,
            Workplace::Hybrid => Scope::Hybrid,
            Workplace::Unknown => Scope::Unknown,
            Workplace::Remote if !self.regions.is_empty() => Scope::Region,
            Workplace::Remote if !self.countries.is_empty() => Scope::Countries,
            Workplace::Remote if self.elsewhere == Elsewhere::Open => Scope::Worldwide,
            Workplace::Remote => Scope::Unknown,
        }
    }
}

/// Eligibility as stored and served: the axes plus the derived `scope`. Rows
/// stored before the axes existed have only `scope`; they read as it says.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct EligibilityWire {
    /// Derived from the fields below; kept for earlier clients.
    scope: Scope,
    #[serde(default)]
    workplace: Option<Workplace>,
    #[serde(default)]
    elsewhere: Option<Elsewhere>,
    #[serde(default)]
    regions: Vec<String>,
    /// ISO 3166-1 alpha-2 codes.
    #[serde(default)]
    countries: Vec<String>,
    onsite_city: Option<String>,
    tz: Option<TzWindow>,
    confidence: f64,
    method: Method,
    evidence: Option<String>,
    /// Original location text, kept for the evidence trail.
    raw: Option<String>,
}

impl Serialize for Eligibility {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        EligibilityWire {
            scope: self.scope(),
            workplace: Some(self.workplace),
            elsewhere: Some(self.elsewhere),
            regions: self.regions.clone(),
            countries: self.countries.clone(),
            onsite_city: self.onsite_city.clone(),
            tz: self.tz.clone(),
            confidence: self.confidence,
            method: self.method,
            evidence: self.evidence.clone(),
            raw: self.raw.clone(),
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for Eligibility {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = EligibilityWire::deserialize(d)?;
        let (workplace, elsewhere) = Eligibility::axes_of(w.scope);
        Ok(Eligibility {
            workplace: w.workplace.unwrap_or(workplace),
            elsewhere: w.elsewhere.unwrap_or(elsewhere),
            regions: w.regions,
            countries: w.countries,
            onsite_city: w.onsite_city,
            tz: w.tz,
            confidence: w.confidence,
            method: w.method,
            evidence: w.evidence,
            raw: w.raw,
        })
    }
}

impl JsonSchema for Eligibility {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Eligibility".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        EligibilityWire::json_schema(generator)
    }
}

/// Pay as the posting states it. Absent when the posting says nothing usable;
/// "competitive" or "DOE" is not pay. Nothing is guessed: a figure is kept
/// only with its currency (and, for base pay, its period), and a figure that
/// fails a check is dropped, not kept wrong (see `normalize::money`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Pay {
    /// Base pay: salary or wage, as stated, in its own currency. Usually one
    /// range; one per level or location when the posting gives several.
    /// Empty when only extras are stated ("competitive salary + equity").
    #[serde(default)]
    pub base: Vec<Money>,
    /// Equity, bonus, commission and the like, one per kind, with figures
    /// when stated.
    #[serde(default)]
    pub extras: Vec<PayExtra>,
    /// Base pay in USD per month, for comparing jobs: the lowest and highest
    /// figures across `base`. Only from USD amounts for now.
    pub usd_month_min: Option<u32>,
    pub usd_month_max: Option<u32>,
    #[serde(default)]
    pub method: PayMethod,
    /// Pay as text: the source's own summary, or one written from its fields.
    pub raw: String,
}

/// An amount of money: one figure (`min == max`), a range, or one end of it
/// ("up to" fills only `max`, "from" only `min`). At least one end is set,
/// both are positive, and `min <= max`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Money {
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// ISO 4217 code, e.g. "USD", "EUR".
    pub currency: String,
    /// What the amount is paid per. Always set for base pay; absent for a
    /// one-off sum (a sign-on bonus, an equity grant).
    pub period: Option<Period>,
    /// What this range is for when a posting gives several: a level
    /// ("Senior Strategic Project Lead") or a location ("UK").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Period {
    Hour,
    Day,
    Week,
    Month,
    Year,
}

impl Period {
    /// How many of this period make a month, by the usual convention
    /// (160 working hours, 21 working days).
    pub fn per_month(self) -> f64 {
        match self {
            Period::Hour => 160.0,
            Period::Day => 21.0,
            Period::Week => 52.0 / 12.0,
            Period::Month => 1.0,
            Period::Year => 1.0 / 12.0,
        }
    }
}

/// Pay on top of the base.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PayExtra {
    pub kind: PayExtraKind,
    /// When stated as money ("$1M in equity", "$20K sign-on").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<Money>,
    /// When stated as a percentage: of the company for equity ("0.25%–1%"),
    /// of base pay for a bonus ("10–15%").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<Percent>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PayExtraKind {
    /// Shares, options, RSUs or tokens.
    Equity,
    Bonus,
    Commission,
    /// On-target earnings: base plus commission at target, as sales roles
    /// state it.
    Ote,
    SignOn,
}

/// A percentage or a range of them, under the same rules as [`Money`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Percent {
    pub min: Option<f64>,
    pub max: Option<f64>,
}

/// Where pay came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PayMethod {
    /// The source's own pay fields.
    #[default]
    Source,
    /// Read from the description; `raw` is the text it came from.
    Text,
}

/// Visa sponsorship as the posting states it. Absent when it says nothing:
/// "must be authorized to work in the US" alone, a citizenship or clearance
/// requirement, or an equal opportunity statement says nothing about it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Visa {
    pub sponsorship: Sponsorship,
    /// The sentence it was read from.
    pub evidence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Sponsorship {
    /// The employer sponsors a visa or work permit, or supports the hire's
    /// immigration process.
    Offered,
    /// Only in some cases: "may", "case by case", transfers only, or only in
    /// some of the posting's locations.
    Limited,
    /// The employer won't sponsor, or the hire must not need sponsorship.
    NotOffered,
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
    /// The source's own category, department or team, as it says it
    /// ("DevOps and Sysadmin", "Engineering; Core Engineering"). Raw input for
    /// the role classifier, which knows which phrases mean something.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub eligibility: Eligibility,
    pub pay: Option<Pay>,
    pub tech_tags: Vec<String>,
    pub description_md: Option<String>,
    pub posted_at: Option<DateTime<Utc>>,
    /// When the source says the listing ends, if it says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// Visa sponsorship as the source states it in a field of its own. Absent
    /// when only the description says; that is read when a job is read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visa: Option<Visa>,
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
    /// Visa sponsorship, read from the description; absent when it says nothing.
    pub visa: Option<Visa>,
    pub tech_tags: Vec<String>,
    pub url: String,
    pub posted_at: Option<DateTime<Utc>>,
    pub first_seen: DateTime<Utc>,
    pub status: String,
    /// What the job is: its role in the vocabulary of `list_roles`, how it
    /// was decided, and modifiers (forward deployed, founding). Absent when
    /// nothing in the title, category or description places it.
    pub role: Option<crate::role::JobRole>,
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
