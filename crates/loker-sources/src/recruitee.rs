use chrono::{DateTime, NaiveDateTime, Utc};
use loker_core::model::{Eligibility, NewJob, Workplace};
use loker_core::normalize::{eligibility_from_text, employment_type, pay};
use serde::Deserialize;
use serde_json::Value;

use crate::{HttpClient, get_json, html_to_md};

pub async fn fetch(client: &HttpClient, board: &str) -> anyhow::Result<Vec<Value>> {
    let host = format!("{}.recruitee.com", board.to_lowercase());
    let body = get_json(client, &crate::host_url(&host, "/api/offers/")).await?;
    let offers = body
        .get("offers")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("expected an `offers` array"))?;
    Ok(offers
        .iter()
        .filter(|o| {
            o.get("status")
                .and_then(Value::as_str)
                .is_none_or(|s| s == "published")
        })
        .cloned()
        .collect())
}

#[derive(Deserialize)]
struct Item {
    id: u64,
    title: String,
    company_name: Option<String>,
    careers_url: Option<String>,
    careers_apply_url: Option<String>,
    description: Option<String>,
    requirements: Option<String>,
    location: Option<String>,
    #[serde(default)]
    locations: Vec<Location>,
    #[serde(default)]
    remote: bool,
    #[serde(default)]
    hybrid: bool,
    #[serde(default)]
    on_site: bool,
    department: Option<String>,
    employment_type_code: Option<String>,
    experience_code: Option<String>,
    salary: Option<Salary>,
    published_at: Option<String>,
    created_at: Option<String>,
    close_at: Option<String>,
}

#[derive(Deserialize)]
struct Location {
    city: Option<String>,
    name: Option<String>,
    country: Option<String>,
}

#[derive(Deserialize)]
struct Salary {
    min: Option<Value>,
    max: Option<Value>,
    currency: Option<String>,
    period: Option<String>,
}

pub fn transform(source_id: &str, board: &str, raw: Value) -> anyhow::Result<NewJob> {
    let item: Item = serde_json::from_value(raw.clone())?;
    let board = board.to_lowercase();
    let canonical = item
        .careers_url
        .clone()
        .unwrap_or_else(|| format!("https://{board}.recruitee.com/o/{}", item.id));

    let places: Vec<String> = item
        .locations
        .iter()
        .filter_map(|l| {
            let city = l.city.as_deref().or(l.name.as_deref());
            let parts: Vec<&str> = [city, l.country.as_deref()]
                .into_iter()
                .flatten()
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .collect();
            (!parts.is_empty()).then(|| parts.join(", "))
        })
        .collect();
    let text = if places.is_empty() {
        item.location.clone().unwrap_or_default()
    } else {
        places.join("; ")
    };
    let eligibility = if item.remote {
        eligibility_from_text(&text, Some(true))
    } else if item.hybrid {
        Eligibility {
            workplace: Workplace::Hybrid,
            ..eligibility_from_text(&text, Some(false))
        }
    } else if item.on_site {
        eligibility_from_text(&text, Some(false))
    } else {
        eligibility_from_text(&text, None)
    };

    let mut html = item.description.unwrap_or_default();
    if let Some(req) = item.requirements.filter(|r| !r.trim().is_empty()) {
        html.push_str("<h3>Requirements</h3>");
        html.push_str(&req);
    }

    Ok(NewJob {
        source_id: source_id.into(),
        source_job_id: item.id.to_string(),
        source_url: canonical.clone(),
        canonical_url: canonical,
        apply_url: item.careers_apply_url,
        title: item.title,
        company: item.company_name.unwrap_or(board),
        seniority: item.experience_code.map(|c| c.replace('_', " ")),
        category: crate::categories(item.department.as_deref()),
        employment_type: item.employment_type_code.as_deref().map(contract),
        eligibility,
        pay: item.salary.and_then(|s| {
            pay(
                s.min.as_ref().and_then(number),
                s.max.as_ref().and_then(number),
                s.currency.as_deref(),
                s.period.as_deref(),
            )
        }),
        tech_tags: Vec::new(),
        description_md: html_to_md(Some(&html))?,
        posted_at: item
            .published_at
            .as_deref()
            .or(item.created_at.as_deref())
            .and_then(timestamp),
        expires_at: item.close_at.as_deref().and_then(timestamp),
        visa: None,
        experience: None,
        raw,
    })
}

fn contract(code: &str) -> String {
    match code.split_once('_').map_or(code, |(kind, _)| kind) {
        "fulltime" => employment_type("full time"),
        "parttime" => employment_type("part time"),
        kind => employment_type(kind),
    }
}

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn timestamp(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.to_utc())
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S UTC")
                .ok()
                .map(|t| t.and_utc())
        })
}
