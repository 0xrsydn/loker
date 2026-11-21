//! Y Combinator's job board (ycombinator.com/jobs), read from its sitemap.
//!
//! The board has no JSON API: every page is Rails + Inertia with the whole
//! payload in one `data-page` attribute, and the listing routes are capped
//! (20 on the landing page, 36-50 per role or location) with pagination
//! parameters ignored. What is complete is `/jobs/sitemap.xml`: one request,
//! about nine thousand URLs, seven thousand of them job pages, each with the
//! company slug. A company's own page (` /companies/<slug>/jobs `) then
//! lists every job it has, uncapped, with the fields below - one request per
//! company instead of one per job.
//!
//! Two things the board states and the postings do not: `visa` (three fixed
//! values) and `salaryRange` (87% of postings, in mixed currencies and
//! periods, sometimes a placeholder). Both are read here rather than from the
//! description, which this route does not fetch.
//!
//! Politeness: the site rate-limits per path (429 after a burst, for minutes,
//! with no `Retry-After`), so pages are fetched one second apart and a 429
//! backs off instead of counting as an answer. Ten percent of the sitemap's
//! job URLs are already gone (404); those companies simply contribute nothing.
//!
//! Coverage: a job is only refreshed when its company page is fetched, and a
//! listing not seen for 30 days closes as stale, so a run must reach every
//! company at least that often. One `pages` unit is `COMPANIES_PER_PAGE`
//! companies; the default three covers 120 of about 1700, so a timer that
//! ingests this source should pass `--pages 50` (2000 companies, the whole
//! board). Smaller runs rotate by day and still get there eventually.
use std::collections::BTreeSet;
use std::time::Duration;

use chrono::{DateTime, Utc};
use loker_core::model::{Money, NewJob, Pay, PayExtra, PayExtraKind, Period, Sponsorship, Visa};
use loker_core::normalize::{self, eligibility_from_text, employment_type};
use serde_json::{Map, Value};

use crate::{HttpClient, as_string, get_text};

pub const SOURCE_ID: &str = "ycombinator";

const HOST: &str = "https://www.ycombinator.com";
const SITEMAP: &str = "/jobs/sitemap.xml";

/// Companies fetched per unit of `pages` (one company page is one request).
const COMPANIES_PER_PAGE: usize = 40;
/// Between two page fetches. The site answers 429 to bursts; one a second held
/// for a whole sweep without a single 429 in testing.
const PAUSE: Duration = Duration::from_secs(1);

pub async fn fetch(client: &HttpClient, pages: u32) -> anyhow::Result<Vec<Value>> {
    let xml = get_text(client, &crate::url(HOST, SITEMAP)).await?;
    let slugs = company_slugs(&xml)?;
    if slugs.is_empty() {
        anyhow::bail!("the sitemap lists no job pages");
    }
    // The sitemap is shuffled and has no dates, so a run that cannot cover it
    // takes the next window by day: successive runs walk the whole board.
    let take = (pages.clamp(1, 50) as usize)
        .saturating_mul(COMPANIES_PER_PAGE)
        .min(slugs.len());
    let start = if take == slugs.len() {
        0
    } else {
        (Utc::now().timestamp() as usize / 86_400).wrapping_mul(take) % slugs.len()
    };
    let mut items = Vec::new();
    for i in 0..take {
        let slug = &slugs[(start + i) % slugs.len()];
        if i > 0 {
            tokio::time::sleep(PAUSE).await;
        }
        match get_page(
            client,
            &crate::url(HOST, &format!("/companies/{slug}/jobs")),
        )
        .await
        {
            Ok(body) => items.extend(page_jobs(&body, slug).unwrap_or_default()),
            Err(e) => tracing::warn!(%slug, "skipping a company page: {e:#}"),
        }
    }
    Ok(items)
}

/// A page of the board, with one more try per 429: a rate limit is a pause,
/// not the answer. `get_text` treats every status as final, which is right
/// for a missing board and wrong here.
async fn get_page(client: &HttpClient, url: &str) -> anyhow::Result<String> {
    let mut pause = Duration::from_secs(20);
    for attempt in 1..=3 {
        let resp = client.get(url).send().await?;
        if resp.status() != reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Ok(resp.error_for_status()?.text().await?);
        }
        if attempt == 3 {
            anyhow::bail!("GET {url}: rate limited three times");
        }
        tracing::warn!(%url, attempt, "rate limited, waiting {pause:?}");
        tokio::time::sleep(pause).await;
        pause *= 2;
    }
    unreachable!()
}

/// Job page paths in the sitemap, as their company slugs. The sitemap also
/// lists role and location pages; those are not jobs.
fn company_slugs(xml: &str) -> anyhow::Result<Vec<String>> {
    let doc = roxmltree::Document::parse(xml)?;
    let mut slugs = BTreeSet::new();
    for node in doc.descendants().filter(|n| n.has_tag_name("loc")) {
        let Some(path) = node.text().map(str::trim) else {
            continue;
        };
        let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
        if let [
            "https:",
            "",
            "www.ycombinator.com",
            "companies",
            slug,
            "jobs",
            _,
        ] = parts.as_slice()
        {
            slugs.insert((*slug).to_string());
        }
    }
    Ok(slugs.into_iter().collect())
}

/// The jobs on one company page: `data-page` holds the Inertia props, with
/// the JSON's quotes HTML-escaped as `&quot;`.
fn page_jobs(html: &str, slug: &str) -> anyhow::Result<Vec<Value>> {
    let raw = data_page(html).ok_or_else(|| anyhow::anyhow!("{slug}: no data-page"))?;
    let page: Value = serde_json::from_str(&raw)?;
    let props = page
        .get("props")
        .ok_or_else(|| anyhow::anyhow!("{slug}: data-page has no props"))?;
    let postings = props
        .get("jobPostings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let company = props.get("company").cloned().unwrap_or(Value::Null);
    let company = Map::from_iter([
        ("slug".to_string(), Value::from(slug)),
        (
            "name".to_string(),
            company.get("name").cloned().unwrap_or(Value::Null),
        ),
        (
            "batch".to_string(),
            company.get("batch_name").cloned().unwrap_or(Value::Null),
        ),
    ]);
    Ok(postings
        .into_iter()
        .map(|job| {
            Value::Object(Map::from_iter([
                ("job".to_string(), job),
                ("company".to_string(), Value::Object(company.clone())),
            ]))
        })
        .collect())
}

/// The HTML-unescaped `data-page` attribute of the component div.
fn data_page(html: &str) -> Option<String> {
    let start = html.find("data-page=\"")? + "data-page=\"".len();
    let rest = &html[start..];
    let end = rest.find('"')?;
    Some(
        rest[..end]
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&"),
    )
}

pub fn transform(raw: Value) -> anyhow::Result<NewJob> {
    let job = raw
        .get("job")
        .ok_or_else(|| anyhow::anyhow!("missing job"))?;
    let s = |k: &str| job.get(k).and_then(as_string).filter(|v| !v.is_empty());
    let id = s("id").ok_or_else(|| anyhow::anyhow!("missing id"))?;
    let title = s("title").ok_or_else(|| anyhow::anyhow!("missing title"))?;
    let company = s("companyName")
        .or_else(|| {
            raw.get("company")
                .and_then(|c| c.get("name"))
                .and_then(as_string)
        })
        .ok_or_else(|| anyhow::anyhow!("missing company"))?;
    let path = s("url").ok_or_else(|| anyhow::anyhow!("missing url"))?;
    let link = format!("{HOST}{path}");
    let location = s("location").unwrap_or_default();
    Ok(NewJob {
        source_id: SOURCE_ID.into(),
        source_job_id: id,
        source_url: link.clone(),
        canonical_url: link.clone(),
        // The board's own apply link is a login redirect; the posting is the
        // honest target.
        apply_url: Some(link),
        title,
        company,
        seniority: None,
        category: crate::categories(
            [s("prettyRole"), s("roleSpecificType")]
                .into_iter()
                .flatten(),
        ),
        employment_type: s("type").as_deref().map(employment_type),
        eligibility: eligibility_from_text(
            &location,
            Some(location.to_lowercase().contains("remote")),
        ),
        pay: pay_of(job),
        tech_tags: job
            .get("skills")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(as_string)
                    .map(|t| t.trim().to_lowercase())
                    .filter(|t| !t.is_empty())
                    .collect()
            })
            .unwrap_or_default(),
        // This route reads the company page, which lists jobs without their
        // descriptions; reading them would cost one request per job.
        description_md: None,
        posted_at: s("createdAt").and_then(|t| age(&t)),
        expires_at: None,
        visa: visa_of(job),
        raw,
    })
}

/// The board's three fixed values. "US citizenship/visa not required" says
/// nothing about sponsorship, so it is not a statement either way.
fn visa_of(job: &Value) -> Option<Visa> {
    let text = job.get("visa").and_then(as_string)?;
    let sponsorship = match text.trim() {
        "Will sponsor" => Sponsorship::Offered,
        "US citizen/visa only" => Sponsorship::NotOffered,
        _ => return None,
    };
    Some(Visa {
        sponsorship,
        evidence: format!("visa: {text}"),
    })
}

fn pay_of(job: &Value) -> Option<Pay> {
    let text = job
        .get("salaryRange")
        .and_then(as_string)
        .unwrap_or_default();
    let text = text.trim();
    let base: Vec<Money> = if text.is_empty() {
        Vec::new()
    } else {
        parse_money(text).into_iter().collect()
    };
    let extras = job
        .get("equityRange")
        .and_then(as_string)
        .and_then(|t| {
            let (min, max) = amounts(&t)?;
            normalize::percent(min, max).map(|percent| PayExtra {
                kind: PayExtraKind::Equity,
                amount: None,
                percent: Some(percent),
            })
        })
        .into_iter()
        .collect();
    normalize::pay_from(base, extras, (!text.is_empty()).then_some(text))
}

/// "$250K - $285K", "15K-20K INR / monthly", "50K-95K EUR", "$60K".
/// The currency is the trailing code when there is one (a lone symbol is USD),
/// and the period is a month only when the text says so. A figure that fails
/// `normalize::money`'s checks (the board's "$1" placeholder) yields nothing.
fn parse_money(text: &str) -> Option<Money> {
    let (min, max) = amounts(text)?;
    let currency = currency(text)?;
    let period = if text.to_lowercase().contains("month") {
        Period::Month
    } else {
        Period::Year
    };
    normalize::money(min, max, Some(&currency), Some(period))
}

/// The figures in a range, in the text's order, scaled by K/M/B. One figure
/// is one amount (min == max), not a range with an open end.
fn amounts(text: &str) -> Option<(Option<f64>, Option<f64>)> {
    let mut found: Vec<f64> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == ',') {
            i += 1;
        }
        let digits: String = chars[start..i].iter().filter(|c| **c != ',').collect();
        let Ok(mut v) = digits.parse::<f64>() else {
            continue;
        };
        // A suffix may be glued on ("250K") or spaced ("2 M").
        let mut j = i;
        while j < chars.len() && chars[j] == ' ' {
            j += 1;
        }
        if let Some(&c) = chars.get(j) {
            let scale = match c.to_ascii_uppercase() {
                'K' => Some(1_000.0),
                'M' => Some(1_000_000.0),
                'B' => Some(1_000_000_000.0),
                _ => None,
            };
            if let Some(scale) = scale {
                v *= scale;
                i = j + 1;
            }
        }
        found.push(v);
        if found.len() == 2 {
            break;
        }
    }
    match found.as_slice() {
        [] => None,
        [one] => Some((Some(*one), Some(*one))),
        [a, b] => Some((Some(*a), Some(*b))),
        _ => None,
    }
}

/// The ISO code the text states: a trailing three-letter code ("INR", "CAD"),
/// else the currency's symbol. A symbol is a guess only for "$", which is USD
/// unless the text names another dollar.
fn currency(text: &str) -> Option<String> {
    let code = text
        .split_whitespace()
        .rfind(|w| w.len() == 3 && w.chars().all(|c| c.is_ascii_uppercase()));
    if let Some(code) = code {
        return Some(code.to_string());
    }
    let symbol = text
        .chars()
        .find(|c| matches!(c, '$' | '€' | '£' | '₹' | '¥'))?;
    Some(
        match symbol {
            '$' => "USD",
            '€' => "EUR",
            '£' => "GBP",
            '₹' => "INR",
            '¥' => "JPY",
            _ => return None,
        }
        .to_string(),
    )
}

/// "about 10 hours", "23 days", "over 1 year", "1 day": the board only gives
/// an age, so the posting date is that age back from now.
fn age(text: &str) -> Option<DateTime<Utc>> {
    let lower = text.to_lowercase();
    let n: f64 = lower
        .split(|c: char| !c.is_ascii_digit())
        .find(|w| !w.is_empty())
        .and_then(|w| w.parse().ok())?;
    let unit = if lower.contains("minute") {
        60.0
    } else if lower.contains("hour") {
        3_600.0
    } else if lower.contains("day") {
        86_400.0
    } else if lower.contains("week") {
        604_800.0
    } else if lower.contains("month") {
        2_592_000.0
    } else if lower.contains("year") {
        31_536_000.0
    } else {
        return None;
    };
    Some(Utc::now() - chrono::Duration::seconds((n * unit) as i64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn job(extra: Value) -> Value {
        let mut base = json!({
            "id": 119313,
            "title": "Founding Research Engineer",
            "url": "/companies/mederva/jobs/243nIzK-founding-research-engineer",
            "companyName": "Mederva",
            "location": "San Francisco, CA, US / Remote (US)",
            "type": "Full-time",
            "prettyRole": "Engineering",
            "roleSpecificType": "Machine learning",
            "visa": "US citizen/visa only",
            "skills": ["Python", "Machine Learning"],
            "createdAt": "23 days",
        });
        base.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        json!({ "job": base, "company": { "slug": "mederva", "name": "Mederva" } })
    }

    #[test]
    fn money_reads_the_boards_shapes() {
        let cases = [
            (
                "$250K - $285K",
                Some((250_000.0, 285_000.0, "USD", Period::Year)),
            ),
            ("$60K", Some((60_000.0, 60_000.0, "USD", Period::Year))),
            (
                "15K-20K INR / monthly",
                Some((15_000.0, 20_000.0, "INR", Period::Month)),
            ),
            (
                "50K-95K EUR",
                Some((50_000.0, 95_000.0, "EUR", Period::Year)),
            ),
            (
                "$100K - $150K CAD",
                Some((100_000.0, 150_000.0, "CAD", Period::Year)),
            ),
            (
                "2M-5M INR",
                Some((2_000_000.0, 5_000_000.0, "INR", Period::Year)),
            ),
            // No currency, no pay.
            ("Competitive", None),
            ("", None),
        ];
        for (text, want) in cases {
            let got = parse_money(text);
            match want {
                None => assert!(got.is_none(), "{text}: expected nothing, got {got:?}"),
                Some((min, max, currency, period)) => {
                    let got = got.unwrap_or_else(|| panic!("{text}: expected pay"));
                    assert_eq!(
                        (got.min, got.max, got.currency.as_str(), got.period),
                        (Some(min), Some(max), currency, Some(period)),
                        "{text}"
                    );
                }
            }
        }
    }

    /// The board's placeholder ("$1") and a range that reads as dollars a
    /// year but is really something else survive parsing and are dropped by
    /// the plausibility check, so a junk figure never becomes pay.
    #[test]
    fn pay_drops_what_cannot_be_a_salary() {
        for text in ["$1", "$220 - $285", "$0"] {
            let j = job(json!({ "salaryRange": text }));
            assert!(
                pay_of(&j["job"]).is_none(),
                "{text} should not be pay: {:?}",
                pay_of(&j["job"])
            );
        }
    }

    #[test]
    fn visa_is_the_boards_own_field() {
        let offered = visa_of(&job(json!({ "visa": "Will sponsor" }))["job"]).expect("offered");
        assert_eq!(offered.sponsorship, Sponsorship::Offered);
        let closed =
            visa_of(&job(json!({ "visa": "US citizen/visa only" }))["job"]).expect("closed");
        assert_eq!(closed.sponsorship, Sponsorship::NotOffered);
        assert!(
            visa_of(&job(json!({ "visa": "US citizenship/visa not required" }))["job"]).is_none()
        );
    }

    #[test]
    fn transform_keeps_the_fields_the_board_states() {
        let j = transform(job(
            json!({ "salaryRange": "$250K - $285K", "equityRange": "0.50% - 1.00%" }),
        ))
        .expect("transform");
        assert_eq!(j.source_job_id, "119313");
        assert_eq!(j.company, "Mederva");
        assert_eq!(
            j.canonical_url,
            "https://www.ycombinator.com/companies/mederva/jobs/243nIzK-founding-research-engineer"
        );
        assert_eq!(j.employment_type.as_deref(), Some("full_time"));
        assert_eq!(j.category.as_deref(), Some("Engineering; Machine learning"));
        assert_eq!(j.tech_tags, ["python", "machine learning"]);
        assert_eq!(
            j.eligibility.raw.as_deref(),
            Some("San Francisco, CA, US / Remote (US)")
        );
        let pay = j.pay.expect("pay");
        assert_eq!(pay.usd_month_max, Some(23_750));
        assert_eq!(pay.extras.len(), 1);
        assert!(j.posted_at.is_some());
    }

    #[test]
    fn sitemap_lists_companies_not_roles() {
        let xml = r#"<?xml version="1.0"?><urlset>
            <url><loc>https://www.ycombinator.com/jobs/role/software-engineer</loc></url>
            <url><loc>https://www.ycombinator.com/companies/acme/jobs/abc-engineer</loc></url>
            <url><loc>https://www.ycombinator.com/companies/globex/jobs/def-designer</loc></url>
            <url><loc>https://www.ycombinator.com/companies/acme/jobs/ghi-other</loc></url>
        </urlset>"#;
        assert_eq!(company_slugs(xml).unwrap(), ["acme", "globex"]);
    }
}
