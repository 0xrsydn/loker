//! Ashby job board API (public, no key):
//! `GET https://api.ashbyhq.com/posting-api/job-board/<board>?includeCompensation=true`.
//!
//! Ashby states `workplaceType` (Remote / OnSite / Hybrid) structurally, so
//! on-site jobs are recognized with high confidence. It has no company-name
//! field; the board slug is used.
use chrono::DateTime;
use loker_core::model::{Eligibility, NewJob, Scope};
use loker_core::normalize::{clean_url, eligibility_from_text, employment_type, pay_from_text};
use serde::Deserialize;
use serde_json::Value;

use crate::{HttpClient, get_json, html_to_md};

pub async fn fetch(client: &HttpClient, board: &str) -> anyhow::Result<Vec<Value>> {
    let url =
        format!("https://api.ashbyhq.com/posting-api/job-board/{board}?includeCompensation=true");
    let body = get_json(client, &url).await?;
    let jobs = body
        .get("jobs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    // Unlisted postings are reachable by link only; don't aggregate them.
    Ok(jobs
        .into_iter()
        .filter(|j| j.get("isListed").and_then(Value::as_bool) != Some(false))
        .collect())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    id: String,
    title: String,
    location: Option<String>,
    #[serde(default)]
    secondary_locations: Vec<Secondary>,
    is_remote: Option<bool>,
    workplace_type: Option<String>,
    job_url: String,
    apply_url: Option<String>,
    published_at: Option<String>,
    description_html: Option<String>,
    employment_type: Option<String>,
    compensation: Option<Compensation>,
}

#[derive(Deserialize)]
struct Secondary {
    location: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Compensation {
    compensation_tier_summary: Option<String>,
}

pub fn transform(source_id: &str, board: &str, raw: Value) -> anyhow::Result<NewJob> {
    let item: Item = serde_json::from_value(raw.clone())?;
    let mut locations: Vec<String> = item.location.iter().cloned().collect();
    locations.extend(
        item.secondary_locations
            .iter()
            .filter_map(|s| s.location.clone()),
    );
    let text = locations.join("; ");
    let eligibility = match item.workplace_type.as_deref() {
        Some("Remote") => eligibility_from_text(&text, Some(true)),
        Some("OnSite") => eligibility_from_text(&text, Some(false)),
        Some("Hybrid") => Eligibility {
            scope: Scope::Hybrid,
            ..eligibility_from_text(&text, Some(false))
        },
        _ => eligibility_from_text(&text, item.is_remote),
    };
    let canonical = clean_url(&item.job_url);
    Ok(NewJob {
        source_id: source_id.into(),
        source_job_id: item.id,
        source_url: canonical.clone(),
        canonical_url: canonical,
        apply_url: item.apply_url.as_deref().map(clean_url),
        title: item.title.trim().to_string(),
        company: board.to_string(),
        seniority: None,
        employment_type: item.employment_type.as_deref().map(employment_type),
        eligibility,
        pay: item
            .compensation
            .and_then(|c| c.compensation_tier_summary)
            .as_deref()
            .and_then(pay_from_text),
        tech_tags: Vec::new(),
        description_md: html_to_md(item.description_html.as_deref())?,
        posted_at: item
            .published_at
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.to_utc()),
        expires_at: None,
        raw,
    })
}
