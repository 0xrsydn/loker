use std::time::Duration;

use chrono::DateTime;
use loker_core::model::{Eligibility, NewJob, Scope, Workplace};
use loker_core::normalize::{eligibility_from_text, employment_type, pay};
use serde::Deserialize;
use serde_json::Value;

use crate::{HttpClient, get_json_within, html_to_md};

const PAGE: usize = 25;
const PAGE_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_PAGES: usize = 400;

pub async fn fetch(client: &HttpClient, site: &str) -> anyhow::Result<Vec<Value>> {
    let mut items = Vec::new();
    for page in 0..MAX_PAGES {
        let url = crate::url(
            "https://api.lever.co",
            &format!(
                "/v0/postings/{site}?mode=json&skip={}&limit={PAGE}",
                page * PAGE
            ),
        );
        let body = get_json_within(client, &url, Some(PAGE_TIMEOUT)).await?;
        let batch = body
            .as_array()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("expected a JSON array of postings"))?;
        let done = batch.len() < PAGE;
        items.extend(batch);
        if done {
            return Ok(items);
        }
    }
    anyhow::bail!(
        "more than {} postings; refusing a partial list",
        PAGE * MAX_PAGES
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    id: String,
    text: String,
    #[serde(default)]
    categories: Categories,
    country: Option<String>,
    workplace_type: Option<String>,
    hosted_url: String,
    apply_url: Option<String>,
    created_at: Option<i64>,
    description: Option<String>,
    #[serde(default)]
    lists: Vec<List>,
    additional: Option<String>,
    salary_range: Option<SalaryRange>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Categories {
    location: Option<String>,
    #[serde(default)]
    all_locations: Vec<String>,
    commitment: Option<String>,
    department: Option<String>,
    team: Option<String>,
}

#[derive(Deserialize)]
struct List {
    text: String,
    content: String,
}

#[derive(Deserialize)]
struct SalaryRange {
    currency: Option<String>,
    interval: Option<String>,
    min: Option<f64>,
    max: Option<f64>,
}

pub fn transform(source_id: &str, site: &str, raw: Value) -> anyhow::Result<NewJob> {
    let item: Item = serde_json::from_value(raw.clone())?;

    let mut locations = item.categories.all_locations.clone();
    if locations.is_empty() {
        locations.extend(item.categories.location.clone());
    }
    let text = locations.join("; ");
    let mut eligibility = match item.workplace_type.as_deref() {
        Some("remote") => eligibility_from_text(&text, Some(true)),
        Some("onsite" | "on-site") => eligibility_from_text(&text, Some(false)),
        Some("hybrid") => Eligibility {
            workplace: Workplace::Hybrid,
            ..eligibility_from_text(&text, Some(false))
        },
        _ => eligibility_from_text(&text, None),
    };
    let physical = matches!(eligibility.scope(), Scope::Onsite | Scope::Hybrid);
    if physical
        && eligibility.countries.is_empty()
        && let Some(c) = item.country.filter(|c| c.len() == 2)
    {
        eligibility.countries = vec![c.to_uppercase()];
    }

    let mut html = item.description.unwrap_or_default();
    for list in &item.lists {
        html.push_str(&format!("<h3>{}</h3><ul>{}</ul>", list.text, list.content));
    }
    html.push_str(item.additional.as_deref().unwrap_or(""));

    Ok(NewJob {
        source_id: source_id.into(),
        source_job_id: item.id,
        source_url: item.hosted_url.clone(),
        canonical_url: item.hosted_url,
        apply_url: item.apply_url,
        title: item.text,
        company: site.to_string(),
        seniority: None,
        category: crate::categories(
            [
                item.categories.department.as_deref(),
                item.categories.team.as_deref(),
            ]
            .into_iter()
            .flatten(),
        ),
        employment_type: item.categories.commitment.as_deref().map(employment_type),
        eligibility,
        pay: item
            .salary_range
            .and_then(|s| pay(s.min, s.max, s.currency.as_deref(), s.interval.as_deref())),
        tech_tags: Vec::new(),
        description_md: html_to_md(Some(&html))?,
        posted_at: item.created_at.and_then(DateTime::from_timestamp_millis),
        expires_at: None,
        visa: None,
        experience: None,
        raw,
    })
}
