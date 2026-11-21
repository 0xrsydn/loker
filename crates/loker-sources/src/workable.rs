//! Workable's job search, across every company that hires through Workable:
//! `GET https://jobs.workable.com/api/v1/jobs?workplace=remote[&location=PH]`.
//! Not a published API: it is the one jobs.workable.com itself uses, so it
//! may change without notice (`loker verify` notices). Remote jobs only, 20 a
//! page, one request a second; `location` takes an ISO country code.
//!
//! Each job states `workplace` (remote, hybrid, on_site) and `locations`
//! (`["TELECOMMUTE", "Berlin, Berlin, Germany"]`). For a remote job the
//! location is the employer's place, not a restriction: in `eval/eligibility`
//! 17 of 60 such jobs said "work from anywhere", "we hire worldwide" or named
//! other places, and the labelers split on what that means. So places the
//! description names win; a description that suggests it is open wider
//! leaves the job unknown (no claim either way); otherwise Workable's place
//! stands. A search is not a complete list, so missing items close nothing;
//! staleness does.
use std::time::Duration;

use anyhow::Context;
use chrono::DateTime;
use loker_core::eligibility_text;
use loker_core::model::{Eligibility, NewJob, Workplace};
use loker_core::normalize::{eligibility_from_text, employment_type};
use serde::Deserialize;
use serde_json::Value;

use crate::html_to_md;

pub const SOURCE_ID: &str = "workable";
const BASE: &str = "https://jobs.workable.com";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    #[serde(default)]
    jobs: Vec<Value>,
    next_page_token: Option<String>,
}

/// Up to `pages` pages of remote jobs, newest first; with `country`, only
/// jobs Workable places in that country.
pub async fn fetch(
    client: &reqwest::Client,
    pages: u32,
    country: Option<&str>,
) -> anyhow::Result<Vec<Value>> {
    let mut items = Vec::new();
    let mut token: Option<String> = None;
    for page in 0..pages {
        if page > 0 {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let mut req = client
            .get(crate::url(BASE, "/api/v1/jobs"))
            .query(&[("workplace", "remote")]);
        if let Some(c) = country {
            req = req.query(&[("location", c)]);
        }
        if let Some(t) = &token {
            req = req.query(&[("pageToken", t)]);
        }
        let body: Page = req
            .send()
            .await?
            .error_for_status()
            .with_context(|| format!("workable page {}", page + 1))?
            .json()
            .await?;
        let done = body.jobs.is_empty() || body.next_page_token.is_none();
        items.extend(body.jobs);
        token = body.next_page_token;
        if done {
            break;
        }
    }
    Ok(items)
}

pub fn transform(raw: Value) -> anyhow::Result<NewJob> {
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    let id = s(&raw, "id").ok_or_else(|| anyhow::anyhow!("missing id"))?;
    let url = s(&raw, "url").ok_or_else(|| anyhow::anyhow!("missing url"))?;
    let title = s(&raw, "title").ok_or_else(|| anyhow::anyhow!("missing title"))?;
    let company = raw
        .get("company")
        .and_then(|c| s(c, "title"))
        .ok_or_else(|| anyhow::anyhow!("missing company"))?;
    // Where: every listed place but the "TELECOMMUTE" marker, else the country.
    let mut places: Vec<String> = raw
        .get("locations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|l| !l.eq_ignore_ascii_case("telecommute"))
        .map(str::to_owned)
        .collect();
    if places.is_empty()
        && let Some(c) = raw.get("location").and_then(|l| s(l, "countryName"))
    {
        places.push(c);
    }
    let text = places.join("; ");
    let description_md = html_to_md(Some(&posting_html(&raw)))?;
    let description = description_md.as_deref().unwrap_or("");
    let eligibility = match s(&raw, "workplace").as_deref() {
        Some("remote") => remote(&text, description),
        Some("on_site") => eligibility_from_text(&text, Some(false)),
        Some("hybrid") => Eligibility {
            workplace: Workplace::Hybrid,
            ..eligibility_from_text(&text, Some(false))
        },
        _ => eligibility_from_text(&text, None),
    };
    Ok(NewJob {
        source_id: SOURCE_ID.into(),
        source_job_id: id,
        source_url: url.clone(),
        canonical_url: url.clone(),
        apply_url: Some(url),
        title,
        company,
        seniority: None,
        category: s(&raw, "department"),
        employment_type: s(&raw, "employmentType").as_deref().map(employment_type),
        eligibility,
        pay: None,
        tech_tags: Vec::new(),
        description_md,
        posted_at: s(&raw, "created")
            .and_then(|d| DateTime::parse_from_rfc3339(&d).ok())
            .map(|t| t.to_utc()),
        expires_at: None,
        raw,
    })
}

/// A remote job: places the description names, else no claim if it says it
/// is open wider, else Workable's place.
fn remote(place: &str, description: &str) -> Eligibility {
    let raw = Some(place.to_string());
    if let Some(s) = eligibility_text::stated(description)
        && !s.only_cities
        && (!s.countries.is_empty() || !s.regions.is_empty())
    {
        let named = [s.regions, s.countries].concat().join("; ");
        return Eligibility {
            raw,
            evidence: Some(format!(
                "description: {}",
                s.evidence.join(" … ").chars().take(280).collect::<String>()
            )),
            ..eligibility_from_text(&named, Some(true))
        };
    }
    // "open to candidates worldwide" (read as worldwide afterwards) or looser.
    let worldwide = eligibility_text::stated(description).is_some_and(|s| s.worldwide);
    if worldwide || eligibility_text::open_wider(description) {
        return Eligibility {
            raw,
            evidence: Some(
                "Workable lists a place, but the posting suggests it is open wider".into(),
            ),
            ..eligibility_from_text("", Some(true))
        };
    }
    eligibility_from_text(place, Some(true))
}

/// The posting in three parts; the requirements often hold where the hire must be.
fn posting_html(raw: &Value) -> String {
    let s = |k: &str| raw.get(k).and_then(Value::as_str).map(str::to_owned);
    [
        ("", "description"),
        ("Requirements", "requirementsSection"),
        ("Benefits", "benefitsSection"),
    ]
    .iter()
    .filter_map(|(heading, k)| {
        let part = s(k).filter(|p| !p.trim().is_empty())?;
        Some(if heading.is_empty() {
            part
        } else {
            format!("<h3>{heading}</h3>{part}")
        })
    })
    .collect()
}
