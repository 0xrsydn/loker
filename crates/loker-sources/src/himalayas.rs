//! Himalayas public jobs API: https://himalayas.app/jobs/api (no key, cursor pagination).
use std::time::Duration;

use anyhow::Context;
use chrono::DateTime;
use loker_core::country;
use loker_core::model::{Eligibility, Method, NewJob, Pay, Scope, TzStrength, TzWindow};
use serde::Deserialize;
use serde_json::Value;

pub const SOURCE_ID: &str = "himalayas";
const API: &str = "https://himalayas.app/jobs/api";
/// Filtered search (e.g. `country=ID` returns worldwide jobs plus jobs listing that country).
const SEARCH_API: &str = "https://himalayas.app/jobs/api/search";
const PAGE_SIZE: u32 = 20;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    #[serde(default)]
    jobs: Vec<Value>,
    next_cursor: Option<String>,
}

/// Fetch up to `pages` pages, newest first, politely (one request per second).
/// With `country`, uses the search endpoint (page-numbered); without, the full feed (cursor).
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
            Some(c) => client.get(SEARCH_API).query(&[
                ("country", c.to_string()),
                ("sort", "recent".into()),
                ("page", (page + 1).to_string()),
            ]),
            None => client.get(API).query(&[("limit", PAGE_SIZE.to_string())]),
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
    employment_type: Option<String>,
    pub_date: Option<i64>,
    description: Option<String>,
}

pub fn transform(raw: Value) -> anyhow::Result<NewJob> {
    let item: Item = serde_json::from_value(raw.clone())?;
    let eligibility = eligibility(&item);
    let pay = pay(&item);
    let description_md = item.description.as_deref().map(htmd::convert).transpose()?;
    Ok(NewJob {
        source_id: SOURCE_ID.into(),
        source_job_id: item.guid.clone(),
        source_url: item.guid.clone(),
        canonical_url: item.guid.clone(),
        apply_url: item.application_link.as_deref().map(strip_tracking),
        title: item.title.trim().to_string(),
        company: item.company_name.trim().to_string(),
        seniority: item.seniority.first().map(|s| s.to_lowercase()),
        employment_type: item.employment_type.as_deref().map(employment_type),
        eligibility,
        pay,
        tech_tags: Vec::new(),
        description_md,
        posted_at: item.pub_date.and_then(|s| DateTime::from_timestamp(s, 0)),
        raw,
    })
}

/// Himalayas states eligibility as data: an empty country list means worldwide, and
/// `timezoneRestrictions` lists the allowed UTC offsets (every zone = no restriction).
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
    let scope = if item.location_restrictions.is_empty() {
        Scope::Worldwide
    } else {
        Scope::Countries
    };
    Eligibility {
        scope,
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

/// USD only for now; other currencies keep the raw text until the FX table lands.
fn pay(item: &Item) -> Option<Pay> {
    let (min, max) = (item.min_salary, item.max_salary);
    if min.is_none() && max.is_none() {
        return None;
    }
    let currency = item.currency.clone().unwrap_or_default();
    let period = item
        .salary_period
        .clone()
        .unwrap_or_else(|| "annual".into());
    let per_month = match period.as_str() {
        "annual" | "yearly" => Some(1.0 / 12.0),
        "monthly" => Some(1.0),
        "weekly" => Some(52.0 / 12.0),
        "daily" => Some(21.0),
        "hourly" => Some(160.0),
        _ => None,
    };
    let usd = |v: Option<f64>| match (currency.as_str(), per_month, v) {
        ("USD", Some(f), Some(v)) => Some((v * f).round() as u32),
        _ => None,
    };
    let fmt = |v: Option<f64>| v.map(|v| format!("{v}")).unwrap_or_else(|| "?".into());
    Some(Pay {
        usd_month_min: usd(min),
        usd_month_max: usd(max),
        raw: format!("{currency} {}–{} {period}", fmt(min), fmt(max)),
    })
}

fn employment_type(s: &str) -> String {
    match s.to_lowercase().as_str() {
        "full time" | "full-time" => "full_time".into(),
        "part time" | "part-time" => "part_time".into(),
        "contractor" | "contract" => "contract".into(),
        "intern" | "internship" => "internship".into(),
        "temporary" => "temporary".into(),
        other => other.replace(' ', "_"),
    }
}

/// Drop `utm_*` and `ref` query parameters so the same posting always has the same URL.
fn strip_tracking(url: &str) -> String {
    let Some((base, query)) = url.split_once('?') else {
        return url.to_string();
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|kv| {
            let key = kv.split('=').next().unwrap_or("");
            !(key.starts_with("utm_") || key == "ref")
        })
        .collect();
    if kept.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", kept.join("&"))
    }
}
