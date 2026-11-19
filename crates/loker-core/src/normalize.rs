//! Normalization shared by every adapter: match keys, URLs, pay, employment
//! type, seniority, and free-text locations → `Eligibility`. Kept deliberately simple;
//! `loker sources eval` measures what these rules miss before new rules are added.
use crate::country;
use crate::model::{
    Eligibility, Method, Money, NewJob, Pay, PayExtra, PayExtraKind, PayMethod, Percent, Period,
    Scope, Seniority,
};

// ---------- every job ----------

/// The cleanups every job gets after its adapter's `transform`, whatever the
/// source: so an adapter only maps fields, and no source can forget one.
///
/// * title, company and category: HTML entities decoded, whitespace collapsed;
/// * every URL through [`clean_url`];
/// * empty optional text (apply URL, seniority, employment type, category,
///   description) becomes `None`;
/// * no pay from the source: pay stated in the description, if any
///   ([`crate::pay_text`]).
pub fn tidy(job: &mut NewJob) {
    job.title = one_line(&decode_entities(&job.title));
    job.company = one_line(&decode_entities(&job.company));
    job.source_url = clean_url(&job.source_url);
    job.canonical_url = clean_url(&job.canonical_url);
    job.apply_url = non_empty(job.apply_url.take()).map(|u| clean_url(&u));
    job.seniority = non_empty(job.seniority.take());
    job.employment_type = non_empty(job.employment_type.take());
    job.category = non_empty(job.category.take()).map(|c| one_line(&decode_entities(&c)));
    job.description_md = non_empty(job.description_md.take());
    if job.pay.is_none() {
        job.pay = job
            .description_md
            .as_deref()
            .and_then(crate::pay_text::pay_from_text);
    }
}

/// Decode the HTML entities job feeds put in plain-text fields. `&amp;` goes
/// last so `&amp;lt;` stays `&lt;`.
pub fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#039;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

// ---------- match keys ----------

/// Legal-form words dropped from the end of company names.
const COMPANY_SUFFIXES: &[&str] = &[
    "inc",
    "llc",
    "ltd",
    "limited",
    "gmbh",
    "corp",
    "corporation",
    "co",
    "plc",
    "pte",
    "pty",
    "sa",
    "sas",
    "bv",
    "ag",
    "oy",
    "ab",
    "srl",
    "sl",
    "kk",
    "tbk",
    "pt",
];

/// "Lemon.io, Inc." → "lemonio". Lowercase alphanumerics only, legal suffixes dropped.
pub fn company_key(name: &str) -> String {
    let mut words: Vec<String> = name
        .to_lowercase()
        .split(|c: char| c.is_whitespace() || c == ',')
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect();
    while words.len() > 1 && COMPANY_SUFFIXES.contains(&words.last().unwrap().as_str()) {
        words.pop();
    }
    words.concat()
}

/// "Expert Talent Pool, Physical AI" → "expert talent pool physical ai".
pub fn title_key(title: &str) -> String {
    title
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------- URLs ----------

/// Drop `utm_*`, `ref` and `source` query parameters and lowercase the host,
/// so the same posting always has the same URL.
pub fn clean_url(url: &str) -> String {
    let url = url.trim();
    let (scheme_host, rest) = match url.find("://") {
        Some(i) => {
            let after = &url[i + 3..];
            let end = after.find(['/', '?', '#']).unwrap_or(after.len());
            (
                format!("{}{}", &url[..i + 3], after[..end].to_lowercase()),
                &after[end..],
            )
        }
        None => (String::new(), url),
    };
    let rest = rest.split('#').next().unwrap_or("");
    let Some((path, query)) = rest.split_once('?') else {
        return format!("{scheme_host}{rest}");
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|kv| {
            let key = kv.split('=').next().unwrap_or("");
            !(key.starts_with("utm_") || key == "ref" || key == "source")
        })
        .collect();
    if kept.is_empty() {
        format!("{scheme_host}{path}")
    } else {
        format!("{scheme_host}{path}?{}", kept.join("&"))
    }
}

// ---------- pay ----------

/// Pay from one range in a source's own fields, the common case.
pub fn pay(
    min: Option<f64>,
    max: Option<f64>,
    currency: Option<&str>,
    period: Option<&str>,
) -> Option<Pay> {
    let period = pay_period(period?)?;
    pay_from(vec![money(min, max, currency, Some(period))?], vec![], None)
}

/// Pay from base ranges and extras. Base ranges without a period, and
/// dollar-like ranges outside 100..=100k a month (a source's typo or a
/// placeholder), are dropped; extras are kept one per kind, the one with figures first. `None`
/// when nothing is left. `raw` is the source's own summary, if it has one.
pub fn pay_from(base: Vec<Money>, mut extras: Vec<PayExtra>, raw: Option<&str>) -> Option<Pay> {
    let base: Vec<Money> = base
        .into_iter()
        .filter(|m| m.period.is_some() && plausible(m))
        .collect();
    extras.sort_by_key(|e| (e.kind, e.amount.is_none() && e.percent.is_none()));
    extras.dedup_by_key(|e| e.kind);
    if base.is_empty() && extras.is_empty() {
        return None;
    }
    let usd: Vec<(Option<f64>, Option<f64>)> = base.iter().filter_map(usd_month).collect();
    let round = |v: f64| v.round() as u32;
    let usd_month_min = usd.iter().filter_map(|u| u.0).reduce(f64::min).map(round);
    let usd_month_max = usd.iter().filter_map(|u| u.1).reduce(f64::max).map(round);
    let raw = match raw.map(str::trim) {
        Some(r) if !r.is_empty() => r.to_string(),
        _ => pay_text(&base, &extras),
    };
    Some(Pay {
        base,
        extras,
        usd_month_min,
        usd_month_max,
        method: PayMethod::Source,
        raw,
    })
}

/// A checked amount: ends that are finite and positive, `min <= max`, a top
/// at most 10× the bottom (wider is a typo in any currency: "65,000–6,000,000"),
/// and an ISO 4217 code. `None` when any check fails: a wrong figure is worse
/// than none. A zero end counts as not stated (sources use 0 for "none").
pub fn money(
    min: Option<f64>,
    max: Option<f64>,
    currency: Option<&str>,
    period: Option<Period>,
) -> Option<Money> {
    let (min, max) = ends(min, max)?;
    if let (Some(a), Some(b)) = (min, max)
        && b > 10.0 * a
    {
        return None;
    }
    let currency = currency?.trim().to_ascii_uppercase();
    if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
        return None;
    }
    Some(Money {
        min,
        max,
        currency,
        period,
        label: None,
    })
}

/// A checked percentage, as [`money`], at most 100.
pub fn percent(min: Option<f64>, max: Option<f64>) -> Option<Percent> {
    let (min, max) = ends(min, max)?;
    [min, max]
        .into_iter()
        .flatten()
        .all(|v| v <= 100.0)
        .then_some(Percent { min, max })
}

fn ends(min: Option<f64>, max: Option<f64>) -> Option<(Option<f64>, Option<f64>)> {
    let ok = |v: Option<f64>| v.filter(|v| v.is_finite() && *v > 0.0);
    match (ok(min), ok(max)) {
        (None, None) => None,
        (Some(a), Some(b)) if a > b => None,
        ends => Some(ends),
    }
}

/// A period as sources write it: "annual", "yearly", "per-year-salary",
/// "1 YEAR", "hr". `None` for anything else ("NONE", "2 WEEK", "biweekly"),
/// so an unknown period is never read as a known one.
pub fn pay_period(s: &str) -> Option<Period> {
    let mut found = None;
    for w in s
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        let p = match w {
            "hour" | "hourly" | "hr" => Period::Hour,
            "day" | "daily" => Period::Day,
            "week" | "weekly" | "wk" => Period::Week,
            "month" | "monthly" | "mo" => Period::Month,
            "year" | "yearly" | "annual" | "annually" | "yr" => Period::Year,
            "1" | "per" | "a" | "salary" | "wage" => continue,
            _ => return None,
        };
        if found.replace(p).is_some_and(|q| q != p) {
            return None;
        }
    }
    found
}

/// Currencies worth roughly a dollar (0.6 to 1.3 USD), so USD's bounds hold
/// for them too. Others can't be checked until there are exchange rates.
const NEAR_USD: &[&str] = &["USD", "EUR", "GBP", "CHF", "CAD", "AUD", "NZD", "SGD"];

/// Base pay in a dollar-like currency must be 100..=100k a month: below is a
/// benefit or a typo ("800 EUR a year"), above a placeholder or a grant.
fn plausible(m: &Money) -> bool {
    let Some(f) = m.period.map(Period::per_month) else {
        return true;
    };
    !NEAR_USD.contains(&m.currency.as_str())
        || [m.min, m.max]
            .into_iter()
            .flatten()
            .all(|v| (100.0..=100_000.0).contains(&(v * f)))
}

/// A USD amount per month, `None` for other currencies or no period.
fn usd_month(m: &Money) -> Option<(Option<f64>, Option<f64>)> {
    let f = m.period?.per_month();
    (m.currency == "USD").then(|| (m.min.map(|v| v * f), m.max.map(|v| v * f)))
}

/// "USD 120,000–150,000 a year + equity, bonus"
fn pay_text(base: &[Money], extras: &[PayExtra]) -> String {
    let base = base.iter().map(money_text).collect::<Vec<_>>().join("; ");
    let extras = extras
        .iter()
        .map(|e| {
            let kind = match e.kind {
                PayExtraKind::Equity => "equity",
                PayExtraKind::Bonus => "bonus",
                PayExtraKind::Commission => "commission",
                PayExtraKind::Ote => "OTE",
                PayExtraKind::SignOn => "sign-on bonus",
            };
            match (&e.amount, &e.percent) {
                (Some(m), _) => format!("{kind} {}", money_text(m)),
                (None, Some(p)) => format!("{kind} {}%", span(p.min, p.max)),
                (None, None) => kind.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    match (base.is_empty(), extras.is_empty()) {
        (false, false) => format!("{base} + {extras}"),
        (false, true) => base,
        _ => extras,
    }
}

fn money_text(m: &Money) -> String {
    let label = m
        .label
        .as_ref()
        .map(|l| format!("{l}: "))
        .unwrap_or_default();
    let per = match m.period {
        Some(Period::Hour) => " an hour",
        Some(Period::Day) => " a day",
        Some(Period::Week) => " a week",
        Some(Period::Month) => " a month",
        Some(Period::Year) => " a year",
        None => "",
    };
    let amount = match (m.min, m.max) {
        (None, Some(_)) => format!("up to {} {}", m.currency, span(m.min, m.max)),
        (Some(_), None) => format!("from {} {}", m.currency, span(m.min, m.max)),
        _ => format!("{} {}", m.currency, span(m.min, m.max)),
    };
    format!("{label}{amount}{per}")
}

/// "120,000–150,000", "120,000", or one end.
fn span(min: Option<f64>, max: Option<f64>) -> String {
    match (min, max) {
        (Some(a), Some(b)) if a == b => number(a),
        (Some(a), Some(b)) => format!("{}–{}", number(a), number(b)),
        (Some(v), None) | (None, Some(v)) => number(v),
        (None, None) => String::new(),
    }
}

/// 120000 → "120,000"; 18.5 → "18.50".
fn number(v: f64) -> String {
    let cents = (v * 100.0).round() as u64;
    let digits = (cents / 100).to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if !cents.is_multiple_of(100) {
        out.push_str(&format!(".{:02}", cents % 100));
    }
    out
}

// ---------- seniority ----------

/// Level words as word sequences, lowest level first.
const LEVEL_WORDS: &[(Seniority, &[&str])] = &[
    (
        Seniority::Intern,
        &[
            "intern",
            "internship",
            "trainee",
            "apprentice",
            "working student",
        ],
    ),
    (
        Seniority::Junior,
        &["junior", "jr", "entry level", "graduate", "new grad"],
    ),
    (Seniority::Mid, &["mid level", "intermediate"]),
    (Seniority::Senior, &["senior", "sr"]),
    (
        Seniority::Lead,
        &["lead", "staff", "principal", "distinguished"],
    ),
    (
        Seniority::Executive,
        &[
            "head of",
            "director",
            "vp",
            "vice president",
            "chief",
            "cto",
            "ceo",
        ],
    ),
];

/// Phrases with a level word that don't name a level.
const NOT_LEVELS: &[&str] = &[
    "member of technical staff",
    "staff accountant",
    "lead generation",
    "lead gen",
];

/// A job's level: from the title when it names one (the employer's own
/// words, and more precise than a source's "senior" for "Staff Engineer"),
/// else from the level the source states. The highest level named wins:
/// "Senior Staff Engineer" is lead. A source's "executive" counts; a title's
/// doesn't ("Account Executive").
pub fn seniority(stated: Option<&str>, title: &str) -> Option<Seniority> {
    level_in(title).or_else(|| {
        let stated = stated?;
        level_in(stated).or_else(|| {
            words(stated)
                .contains(" executive ")
                .then_some(Seniority::Executive)
        })
    })
}

fn level_in(text: &str) -> Option<Seniority> {
    let mut t = words(text);
    for p in NOT_LEVELS {
        t = t.replace(&format!(" {p} "), " ");
    }
    LEVEL_WORDS
        .iter()
        .rev()
        .find(|(_, ws)| ws.iter().any(|w| t.contains(&format!(" {w} "))))
        .map(|(level, _)| *level)
}

/// Lowercase alphanumeric words joined by single spaces, padded with a space
/// on each side so phrases match whole words.
fn words(text: &str) -> String {
    let lower = text.to_lowercase();
    let ws: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    format!(" {} ", ws.join(" "))
}

// ---------- employment type ----------

pub fn employment_type(s: &str) -> String {
    let k: String = s
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    match k.as_str() {
        "fulltime" => "full_time".into(),
        "parttime" => "part_time".into(),
        "contractor" | "contract" | "freelance" => "contract".into(),
        "intern" | "internship" => "internship".into(),
        "temporary" | "temp" => "temporary".into(),
        _ => s.trim().to_lowercase().replace([' ', '-'], "_"),
    }
}

// ---------- locations ----------

const WORLDWIDE: &[&str] = &[
    "anywhere",
    "worldwide",
    "global",
    "anywhere in the world",
    "world",
    "remote anywhere",
];

const REGIONS: &[(&str, &str)] = &[
    ("europe", "Europe"),
    ("eu", "Europe"),
    ("emea", "EMEA"),
    ("apac", "APAC"),
    ("asia pacific", "APAC"),
    ("asia", "Asia"),
    ("southeast asia", "Southeast Asia"),
    ("sea", "Southeast Asia"),
    ("latam", "LATAM"),
    ("latin america", "LATAM"),
    ("americas", "Americas"),
    ("north america", "North America"),
    ("south america", "South America"),
    ("africa", "Africa"),
    ("middle east", "Middle East"),
    ("oceania", "Oceania"),
];

/// Words that say "remote" without saying where.
const NOISE: &[&str] = &[
    "remote",
    "only",
    "based",
    "fully",
    "100%",
    "first",
    "work from home",
    "wfh",
];

/// Turn a free-text location ("Remote - US", "Hong Kong; Singapore", "EMEA",
/// "Anywhere") into eligibility.
///
/// `remote` is what the source states structurally: `Some(true)` for a
/// remote-only board or an explicit remote flag, `Some(false)` for an explicit
/// on-site flag, `None` when the source doesn't say. Text containing "remote"
/// counts as `Some(true)`.
pub fn eligibility_from_text(text: &str, remote: Option<bool>) -> Eligibility {
    let raw = text.trim();
    let lower = raw.to_lowercase();
    let remote = remote.or_else(|| lower.contains("remote").then_some(true));

    let mut countries = Vec::new();
    let mut regions = Vec::new();
    let mut worldwide = false;
    let mut unmapped = Vec::new();
    for part in lower
        .split([';', ',', '/', '|', '(', ')'])
        .flat_map(|p| p.split(" or "))
    {
        let mut p = part
            .trim()
            .trim_matches(|c: char| c == '-' || c == '–')
            .trim()
            .to_string();
        if WORLDWIDE.contains(&p.as_str()) {
            worldwide = true;
            continue;
        }
        for n in NOISE {
            p = p.replace(n, " ");
        }
        let p = p.split_whitespace().collect::<Vec<_>>().join(" ");
        let p = p
            .trim_matches(|c: char| c == '-' || c == '–' || c == ':')
            .trim();
        if p.is_empty() {
            continue;
        }
        if WORLDWIDE.contains(&p) {
            worldwide = true;
        } else if let Some((_, r)) = REGIONS.iter().find(|(k, _)| *k == p) {
            regions.push(r.to_string());
        } else if let Some(c) = country::to_alpha2(p) {
            countries.push(c.to_string());
        } else {
            unmapped.push(p.to_string());
        }
    }
    countries.sort();
    countries.dedup();
    regions.sort();
    regions.dedup();

    let unmapped_note =
        (!unmapped.is_empty()).then(|| format!("unmapped: {}", unmapped.join(", ")));
    let base = |scope, confidence: f64, evidence: Option<String>| Eligibility {
        scope,
        regions: Vec::new(),
        countries: Vec::new(),
        onsite_city: None,
        tz: None,
        confidence,
        method: Method::Rules,
        evidence,
        raw: (!raw.is_empty()).then(|| raw.to_string()),
    };

    match remote {
        Some(false) => Eligibility {
            onsite_city: (!raw.is_empty()).then(|| raw.to_string()),
            countries,
            ..base(Scope::Onsite, 0.85, None)
        },
        Some(true) => {
            if !regions.is_empty() {
                Eligibility {
                    regions,
                    countries,
                    ..base(Scope::Region, 0.7, unmapped_note)
                }
            } else if !countries.is_empty() {
                let conf = if unmapped.is_empty() { 0.75 } else { 0.6 };
                Eligibility {
                    countries,
                    ..base(Scope::Countries, conf, unmapped_note)
                }
            } else if worldwide {
                base(Scope::Worldwide, 0.8, None)
            } else {
                let why = unmapped_note.unwrap_or_else(|| "remote, no region stated".into());
                base(Scope::Unknown, 0.3, Some(why))
            }
        }
        None if raw.is_empty() => base(Scope::Unknown, 0.2, Some("no location given".into())),
        // Remote or on-site isn't stated, but the places are: whichever it is,
        // the job is tied to them. Scope stays unknown; the verdict uses them.
        None if !countries.is_empty() || !regions.is_empty() => {
            let places = [regions.as_slice(), countries.as_slice()]
                .concat()
                .join(", ");
            Eligibility {
                regions,
                countries,
                ..base(
                    Scope::Unknown,
                    0.5,
                    Some(format!("lists {places}; doesn't say remote or on-site")),
                )
            }
        }
        None => base(
            Scope::Unknown,
            0.3,
            Some(format!("location \"{raw}\" doesn't say remote or on-site")),
        ),
    }
}
