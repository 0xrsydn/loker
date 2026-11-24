use std::time::Duration;

use anyhow::Context;
use chrono::DateTime;
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
        Some("remote") => crate::remote_listed("Workable", &text, description, false),
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
        visa: None,
        experience: None,
        raw,
    })
}

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
