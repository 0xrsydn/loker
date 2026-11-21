use std::time::Duration;

use anyhow::Context;
use chrono::DateTime;
use loker_core::country;
use loker_core::model::{
    Eligibility, Elsewhere, Method, NewJob, Pay, TzStrength, TzWindow, Workplace,
};
use loker_core::normalize;
use serde::Deserialize;
use serde_json::Value;

pub const SOURCE_ID: &str = "himalayas";
const BASE: &str = "https://himalayas.app";
const PAGE_SIZE: u32 = 20;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    #[serde(default)]
    jobs: Vec<Value>,
    next_cursor: Option<String>,
}

pub async fn fetch(
    client: &reqwest::Client,
    pages: u32,
    country: Option<&str>,
) -> anyhow::Result<Vec<Value>> {
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    for page in 0..pages {
        if page > 0 {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let mut req = match country {
            Some(c) => client.get(crate::url(BASE, "/jobs/api/search")).query(&[
                ("country", c.to_string()),
                ("sort", "recent".into()),
                ("page", (page + 1).to_string()),
            ]),
            None => client
                .get(crate::url(BASE, "/jobs/api"))
                .query(&[("limit", PAGE_SIZE.to_string())]),
        };
        if let Some(c) = &cursor {
            req = req.query(&[("cursor", c)]);
        }
        let body: Page = req
            .send()
            .await?
            .error_for_status()
            .with_context(|| format!("himalayas page {}", page + 1))?
            .json()
            .await?;
        let done = body.jobs.is_empty() || (country.is_none() && body.next_cursor.is_none());
        items.extend(body.jobs);
        cursor = body.next_cursor;
        if done {
            break;
        }
    }
    Ok(items)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    guid: String,
    title: String,
    company_name: String,
    application_link: Option<String>,
    #[serde(default)]
    location_restrictions: Vec<String>,
    #[serde(default)]
    timezone_restrictions: Vec<f64>,
    min_salary: Option<f64>,
    max_salary: Option<f64>,
    currency: Option<String>,
    salary_period: Option<String>,
    #[serde(default)]
    seniority: Vec<String>,
    #[serde(default)]
    parent_categories: Vec<String>,
    employment_type: Option<String>,
    pub_date: Option<i64>,
    expiry_date: Option<i64>,
    description: Option<String>,
}

pub fn transform(raw: Value) -> anyhow::Result<NewJob> {
    let item: Item = serde_json::from_value(raw.clone())?;
    let eligibility = eligibility(&item);
    let pay = pay(&item);
    let description_md = crate::html_to_md(item.description.as_deref())?;
    Ok(NewJob {
        source_id: SOURCE_ID.into(),
        source_job_id: item.guid.clone(),
        source_url: item.guid.clone(),
        canonical_url: item.guid.clone(),
        apply_url: item.application_link,
        title: item.title,
        company: item.company_name,
        seniority: item.seniority.first().map(|s| s.to_lowercase()),
        category: crate::categories(&item.parent_categories),
        employment_type: item
            .employment_type
            .as_deref()
            .map(normalize::employment_type),
        eligibility,
        pay,
        tech_tags: Vec::new(),
        description_md,
        posted_at: item.pub_date.and_then(|s| DateTime::from_timestamp(s, 0)),
        expires_at: item
            .expiry_date
            .and_then(|s| DateTime::from_timestamp(s, 0)),
        visa: None,
        experience: None,
        raw,
    })
}

fn eligibility(item: &Item) -> Eligibility {
    let mut countries: Vec<String> = Vec::new();
    let mut unmapped: Vec<&str> = Vec::new();
    for name in &item.location_restrictions {
        match country::to_alpha2(name) {
            Some(code) => countries.push(code.to_string()),
            None => unmapped.push(name),
        }
    }
    countries.sort();
    countries.dedup();
    let elsewhere = if item.location_restrictions.is_empty() {
        Elsewhere::Open
    } else {
        Elsewhere::Excluded
    };
    Eligibility {
        workplace: Workplace::Remote,
        elsewhere,
        regions: Vec::new(),
        countries,
        onsite_city: None,
        tz: TzWindow::from_offsets(&item.timezone_restrictions, TzStrength::Required),
        confidence: if unmapped.is_empty() { 0.95 } else { 0.8 },
        method: Method::Source,
        evidence: (!unmapped.is_empty())
            .then(|| format!("unmapped countries: {}", unmapped.join(", "))),
        raw: (!item.location_restrictions.is_empty())
            .then(|| item.location_restrictions.join(", ")),
    }
}

fn pay(item: &Item) -> Option<Pay> {
    normalize::pay(
        item.min_salary,
        item.max_salary,
        item.currency.as_deref(),
        item.salary_period.as_deref(),
    )
}
