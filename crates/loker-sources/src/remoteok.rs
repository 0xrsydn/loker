//! Remote OK API (public, no key): `GET https://remoteok.com/api`.
//!
//! Terms: link back to the listing on Remote OK and name it as the source; the
//! first array element is the legal notice, not a job. Salaries are USD per
//! year, with 0 meaning "not given" and occasional junk values (caught by the
//! pay sanity bounds).
use chrono::DateTime;
use loker_core::model::NewJob;
use loker_core::normalize::{clean_url, eligibility_from_text, pay};
use serde_json::Value;

use crate::{HttpClient, as_f64, as_string, decode_entities, get_json, html_to_md};

pub const SOURCE_ID: &str = "remoteok";

pub async fn fetch(client: &HttpClient) -> anyhow::Result<Vec<Value>> {
    let body = get_json(client, &crate::url("https://remoteok.com", "/api")).await?;
    Ok(body
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|j| j.get("id").is_some())
                .cloned()
                .collect()
        })
        .unwrap_or_default())
}

pub fn transform(raw: Value) -> anyhow::Result<NewJob> {
    let s = |k: &str| raw.get(k).and_then(as_string);
    let id = s("id").ok_or_else(|| anyhow::anyhow!("missing id"))?;
    let url = clean_url(&s("url").ok_or_else(|| anyhow::anyhow!("missing url"))?);
    let title = s("position").ok_or_else(|| anyhow::anyhow!("missing position"))?;
    let company = s("company").ok_or_else(|| anyhow::anyhow!("missing company"))?;
    let tags = raw
        .get("tags")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(as_string)
                .map(|t| t.to_lowercase())
                .collect()
        })
        .unwrap_or_default();
    Ok(NewJob {
        source_id: SOURCE_ID.into(),
        source_job_id: id,
        source_url: url.clone(),
        canonical_url: url.clone(),
        apply_url: s("apply_url").map(|u| clean_url(&u)).or(Some(url)),
        title: decode_entities(title.trim()),
        company: decode_entities(company.trim()),
        seniority: None,
        employment_type: None,
        eligibility: eligibility_from_text(
            &decode_entities(&s("location").unwrap_or_default()),
            Some(true),
        ),
        pay: pay(
            as_f64(raw.get("salary_min")),
            as_f64(raw.get("salary_max")),
            Some("USD"),
            Some("annual"),
        ),
        tech_tags: tags,
        description_md: html_to_md(s("description").as_deref())?,
        posted_at: s("date")
            .and_then(|d| DateTime::parse_from_rfc3339(&d).ok())
            .map(|t| t.to_utc()),
        expires_at: None,
        raw,
    })
}
