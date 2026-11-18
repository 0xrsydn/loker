//! Normalization shared by every adapter: match keys, URLs, pay, employment
//! type, and free-text locations → `Eligibility`. Kept deliberately simple;
//! `loker sources eval` measures what these rules miss before new rules are added.
use crate::country;
use crate::model::{Eligibility, Method, Pay, Scope};

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

/// Build `Pay` from a source's numbers. USD amounts are converted to USD per
/// month; anything implausible (outside 100..=100k USD/month) keeps only the
/// raw text, so a bad source value never reaches pay sorting.
pub fn pay(
    min: Option<f64>,
    max: Option<f64>,
    currency: Option<&str>,
    period: Option<&str>,
) -> Option<Pay> {
    let nonzero = |v: Option<f64>| v.filter(|v| *v > 0.0);
    let (min, max) = (nonzero(min), nonzero(max));
    if min.is_none() && max.is_none() {
        return None;
    }
    let currency = currency.unwrap_or("").trim().to_uppercase();
    let period = period.unwrap_or("annual").trim().to_lowercase();
    let per_month = match period.as_str() {
        "annual" | "annually" | "yearly" | "year" | "yr" => Some(1.0 / 12.0),
        "monthly" | "month" | "mo" => Some(1.0),
        "weekly" | "week" | "wk" => Some(52.0 / 12.0),
        "daily" | "day" => Some(21.0),
        "hourly" | "hour" | "hr" => Some(160.0),
        _ => None,
    };
    let usd = |v: Option<f64>| match (currency.as_str(), per_month, v) {
        ("USD", Some(f), Some(v)) => {
            let m = (v * f).round();
            (100.0..=100_000.0).contains(&m).then_some(m as u32)
        }
        _ => None,
    };
    let fmt = |v: Option<f64>| v.map(|v| format!("{v}")).unwrap_or_else(|| "?".into());
    Some(Pay {
        usd_month_min: usd(min),
        usd_month_max: usd(max),
        raw: format!("{currency} {}–{} {period}", fmt(min), fmt(max))
            .trim()
            .to_string(),
    })
}

/// Parse a display range like "$130K – $225K" or "$40 – $60 / hr". Dollar
/// amounts only; anything else is kept as raw text with no USD figures.
pub fn pay_from_text(text: &str) -> Option<Pay> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let lower = t.to_lowercase();
    let period = if lower.contains("/hr") || lower.contains("hour") {
        "hourly"
    } else if lower.contains("month") {
        "monthly"
    } else {
        "annual"
    };
    let amounts: Vec<f64> = t
        .split(['–', '-', '—'])
        .filter_map(|part| {
            let p = part.trim();
            let digits = p.strip_prefix('$')?;
            let num: String = digits
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
                .filter(|c| *c != ',')
                .collect();
            let v: f64 = num.parse().ok()?;
            let suffix = digits[num.len()..]
                .chars()
                .next()
                .map(|c| c.to_ascii_uppercase());
            Some(match suffix {
                Some('K') => v * 1e3,
                Some('M') => v * 1e6,
                _ => v,
            })
        })
        .collect();
    let mut p = match amounts.as_slice() {
        [a, b, ..] => pay(Some(*a), Some(*b), Some("USD"), Some(period)),
        [a] => pay(Some(*a), Some(*a), Some("USD"), Some(period)),
        [] => Some(Pay {
            usd_month_min: None,
            usd_month_max: None,
            raw: String::new(),
        }),
    }?;
    p.raw = t.to_string();
    Some(p)
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
        None => base(
            Scope::Unknown,
            0.3,
            Some(format!("location \"{raw}\" doesn't say remote or on-site")),
        ),
    }
}
