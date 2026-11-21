//! Greenhouse job board API (public, no key):
//! `GET https://boards-api.greenhouse.io/v1/boards/<board>/jobs?content=true`.
//!
//! Location is free text and Greenhouse has no remote flag, so eligibility
//! comes from the text rules; "Hong Kong; Singapore" stays `unknown` because it
//! doesn't say remote or on-site.
use chrono::DateTime;
use loker_core::model::NewJob;
use loker_core::normalize::{decode_entities, eligibility_from_text};
use serde::Deserialize;
use serde_json::Value;

use crate::{HttpClient, get_json, html_to_md};

pub async fn fetch(client: &HttpClient, board: &str) -> anyhow::Result<Vec<Value>> {
    let url = crate::url(
        "https://boards-api.greenhouse.io",
        &format!("/v1/boards/{board}/jobs?content=true"),
    );
    let body = get_json(client, &url).await?;
    Ok(body
        .get("jobs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

#[derive(Deserialize)]
struct Item {
    id: u64,
    title: String,
    absolute_url: Option<String>,
    location: Option<Location>,
    company_name: Option<String>,
    first_published: Option<String>,
    updated_at: Option<String>,
    content: Option<String>,
    #[serde(default)]
    departments: Vec<Location>,
}

/// Anything with an optional `name` (a location, a department).
#[derive(Deserialize)]
struct Location {
    name: Option<String>,
}

pub fn transform(source_id: &str, board: &str, raw: Value) -> anyhow::Result<NewJob> {
    let item: Item = serde_json::from_value(raw.clone())?;
    let board = board.to_lowercase();
    // Identity from board + id: `absolute_url` varies (EU host, company careers page with ?gh_jid=).
    let canonical = format!("https://job-boards.greenhouse.io/{board}/jobs/{}", item.id);
    let location = item.location.and_then(|l| l.name).unwrap_or_default();
    let posted = item.first_published.or(item.updated_at);
    // Greenhouse sends the description as escaped HTML (`&lt;p&gt;`).
    let content = item.content.as_deref().map(decode_entities);
    Ok(NewJob {
        source_id: source_id.into(),
        source_job_id: item.id.to_string(),
        source_url: item
            .absolute_url
            .clone()
            .unwrap_or_else(|| canonical.clone()),
        canonical_url: canonical,
        apply_url: item.absolute_url,
        title: item.title,
        company: item.company_name.unwrap_or(board),
        seniority: None,
        category: crate::categories(item.departments.iter().filter_map(|d| d.name.as_deref())),
        employment_type: None,
        eligibility: eligibility_from_text(&location, None),
        pay: None,
        tech_tags: Vec::new(),
        description_md: html_to_md(content.as_deref())?,
        posted_at: posted
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.to_utc()),
        expires_at: None,
        visa: None,
        experience: None,
        raw,
    })
}
