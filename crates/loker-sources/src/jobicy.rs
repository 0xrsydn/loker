//! Jobicy remote jobs API (public, no key): `GET https://jobicy.com/api/v2/remote-jobs?count=100`.
//!
//! Terms: credit Jobicy with a link to the listing, and send applicants to the
//! job URL from the feed. Jobicy is remote-only, so `jobGeo` is where the
//! remote job is open ("USA", "Europe", "Anywhere").
use chrono::DateTime;
use loker_core::model::NewJob;
use loker_core::normalize::{decode_entities, eligibility_from_text, employment_type, pay};
use serde_json::Value;

use crate::{HttpClient, as_f64, as_string, get_json, html_to_md};

pub const SOURCE_ID: &str = "jobicy";

pub async fn fetch(client: &HttpClient) -> anyhow::Result<Vec<Value>> {
    let body = get_json(
        client,
        &crate::url("https://jobicy.com", "/api/v2/remote-jobs?count=100"),
    )
    .await?;
    Ok(body
        .get("jobs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

pub fn transform(raw: Value) -> anyhow::Result<NewJob> {
    let s = |k: &str| raw.get(k).and_then(as_string);
    let id = s("id").ok_or_else(|| anyhow::anyhow!("missing id"))?;
    let url = s("url").ok_or_else(|| anyhow::anyhow!("missing url"))?;
    let title = s("jobTitle").ok_or_else(|| anyhow::anyhow!("missing jobTitle"))?;
    let company = s("companyName").ok_or_else(|| anyhow::anyhow!("missing companyName"))?;
    // jobType arrives as a list or as a string like "['Full-Time']".
    let job_type = match raw.get("jobType") {
        Some(Value::Array(a)) => a.first().and_then(as_string),
        Some(Value::String(t)) => t
            .trim_matches(|c| c == '[' || c == ']')
            .split(',')
            .next()
            .map(|x| x.trim().trim_matches(['\'', '"']).to_string()),
        _ => None,
    }
    .filter(|t| !t.is_empty());
    Ok(NewJob {
        source_id: SOURCE_ID.into(),
        source_job_id: id,
        source_url: url.clone(),
        canonical_url: url.clone(),
        apply_url: Some(url),
        title,
        company,
        // "Director" is set on account executives and BDRs as often as on
        // directors, so it says nothing; the title decides instead.
        seniority: s("jobLevel")
            .map(|l| l.to_lowercase())
            .filter(|l| l != "any" && l != "director"),
        // jobIndustry is a list, like jobType.
        category: match raw.get("jobIndustry") {
            Some(Value::Array(a)) => crate::categories(a.iter().filter_map(as_string)),
            Some(v) => as_string(v),
            None => None,
        },
        employment_type: job_type.as_deref().map(employment_type),
        eligibility: eligibility_from_text(
            &decode_entities(&s("jobGeo").unwrap_or_default()),
            Some(true),
        ),
        pay: pay(
            as_f64(raw.get("salaryMin")),
            as_f64(raw.get("salaryMax")),
            s("salaryCurrency").as_deref(),
            s("salaryPeriod").as_deref(),
        ),
        tech_tags: Vec::new(),
        description_md: html_to_md(s("jobDescription").as_deref())?,
        posted_at: s("pubDate")
            .and_then(|d| DateTime::parse_from_rfc3339(&d).ok())
            .map(|t| t.to_utc()),
        expires_at: None,
        visa: None,
        experience: None,
        raw,
    })
}
