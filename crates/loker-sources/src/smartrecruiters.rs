use std::collections::HashMap;
use std::sync::Arc;

use chrono::DateTime;
use loker_core::model::{Eligibility, NewJob, Workplace};
use loker_core::normalize::{eligibility_from_text, employment_type};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::{HttpClient, get_json, get_json_unless_missing, html_to_md};

const PAGE: usize = 100;
const MAX_PAGES: usize = 200;
const DETAILS_IN_FLIGHT: usize = 4;
const DETAILS_WITHOUT_STORE: usize = 20;
const SAME_POSTING: [&str; 6] = [
    "id",
    "name",
    "releasedDate",
    "jobAdId",
    "refNumber",
    "location",
];

pub async fn fetch(
    client: &HttpClient,
    company: &str,
    stored: Option<HashMap<String, Value>>,
) -> anyhow::Result<Vec<Value>> {
    let company = crate::segment(company);
    let mut listed = Vec::new();
    for page in 0..MAX_PAGES {
        let url = crate::url(
            "https://api.smartrecruiters.com",
            &format!(
                "/v1/companies/{company}/postings?limit={PAGE}&offset={}",
                page * PAGE
            ),
        );
        let body = get_json(client, &url).await?;
        let batch = body
            .get("content")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("expected a `content` array of postings"))?;
        let done = batch.len() < PAGE;
        listed.extend(
            batch
                .iter()
                .filter(|p| p.get("visibility").and_then(Value::as_str) != Some("INTERNAL"))
                .cloned(),
        );
        if done {
            return details(client, &company, listed, stored).await;
        }
    }
    anyhow::bail!(
        "more than {} postings; refusing a partial list",
        PAGE * MAX_PAGES
    )
}

async fn details(
    client: &HttpClient,
    company: &str,
    listed: Vec<Value>,
    stored: Option<HashMap<String, Value>>,
) -> anyhow::Result<Vec<Value>> {
    let listed: Vec<Value> = match &stored {
        Some(_) => listed,
        None => listed.into_iter().take(DETAILS_WITHOUT_STORE).collect(),
    };
    let stored = stored.unwrap_or_default();
    let mut out: Vec<Option<Value>> = vec![None; listed.len()];
    let slots = Arc::new(Semaphore::new(DETAILS_IN_FLIGHT));
    let mut tasks = JoinSet::new();
    for (i, posting) in listed.into_iter().enumerate() {
        let Some(id) = posting.get("id").and_then(Value::as_str).map(String::from) else {
            continue;
        };
        if let Some(kept) = stored.get(&id).filter(|s| unchanged(&posting, s)) {
            out[i] = Some(kept.clone());
            continue;
        }
        let client = client.clone();
        let slots = slots.clone();
        let url = crate::url(
            "https://api.smartrecruiters.com",
            &format!("/v1/companies/{company}/postings/{}", crate::segment(&id)),
        );
        tasks.spawn(async move {
            let _slot = slots.acquire_owned().await;
            (i, get_json_unless_missing(&client, &url).await)
        });
    }
    while let Some(done) = tasks.join_next().await {
        let (i, detail) = done?;
        out[i] = detail?.filter(|d| d.get("active").and_then(Value::as_bool) != Some(false));
    }
    Ok(out.into_iter().flatten().collect())
}

fn unchanged(listed: &Value, stored: &Value) -> bool {
    stored.get("jobAd").is_some() && SAME_POSTING.iter().all(|k| listed.get(k) == stored.get(k))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    id: String,
    name: String,
    company: Option<Company>,
    location: Option<Location>,
    released_date: Option<String>,
    posting_url: Option<String>,
    apply_url: Option<String>,
    department: Option<Label>,
    function: Option<Label>,
    type_of_employment: Option<Label>,
    experience_level: Option<Label>,
    job_ad: Option<JobAd>,
}

#[derive(Deserialize)]
struct Company {
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Location {
    city: Option<String>,
    region: Option<String>,
    country: Option<String>,
    full_location: Option<String>,
    #[serde(default)]
    remote: bool,
    #[serde(default)]
    hybrid: bool,
}

#[derive(Deserialize)]
struct Label {
    id: Option<Value>,
    label: Option<String>,
}

#[derive(Deserialize)]
struct JobAd {
    sections: Option<Sections>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sections {
    company_description: Option<Section>,
    job_description: Option<Section>,
    qualifications: Option<Section>,
    additional_information: Option<Section>,
}

#[derive(Deserialize)]
struct Section {
    title: Option<String>,
    text: Option<String>,
}

pub fn transform(source_id: &str, company: &str, raw: Value) -> anyhow::Result<NewJob> {
    let item: Item = serde_json::from_value(raw.clone())?;
    let canonical = item.posting_url.clone().unwrap_or_else(|| {
        format!(
            "https://jobs.smartrecruiters.com/{company}/{}",
            crate::segment(&item.id)
        )
    });

    let place = item.location.as_ref().map(|l| {
        l.full_location.clone().unwrap_or_else(|| {
            [l.city.as_deref(), l.region.as_deref(), l.country.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(", ")
        })
    });
    let text = place.unwrap_or_default();
    let eligibility = match &item.location {
        Some(l) if l.remote => eligibility_from_text(&text, Some(true)),
        Some(l) if l.hybrid => Eligibility {
            workplace: Workplace::Hybrid,
            ..eligibility_from_text(&text, Some(false))
        },
        Some(_) => eligibility_from_text(&text, Some(false)),
        None => eligibility_from_text(&text, None),
    };

    let mut html = String::new();
    if let Some(s) = item.job_ad.and_then(|a| a.sections) {
        for section in [
            s.job_description,
            s.qualifications,
            s.additional_information,
            s.company_description,
        ]
        .into_iter()
        .flatten()
        {
            let Some(body) = section.text.filter(|t| !t.trim().is_empty()) else {
                continue;
            };
            if let Some(title) = section.title.filter(|t| !t.trim().is_empty()) {
                html.push_str(&format!("<h3>{title}</h3>"));
            }
            html.push_str(&body);
        }
    }

    let label = |l: &Option<Label>| {
        l.as_ref()
            .and_then(|l| l.label.clone())
            .filter(|s| !s.is_empty())
    };
    Ok(NewJob {
        source_id: source_id.into(),
        source_job_id: item.id,
        source_url: canonical.clone(),
        canonical_url: canonical,
        apply_url: item.apply_url,
        title: item.name,
        company: item
            .company
            .and_then(|c| c.name)
            .unwrap_or_else(|| company.to_string()),
        seniority: item
            .experience_level
            .as_ref()
            .and_then(|l| l.id.as_ref()?.as_str())
            .and_then(level)
            .map(String::from),
        category: crate::categories(
            [label(&item.department), label(&item.function)]
                .iter()
                .flatten()
                .map(String::as_str),
        ),
        employment_type: item
            .type_of_employment
            .as_ref()
            .and_then(|l| l.label.as_deref())
            .map(employment_type),
        eligibility,
        pay: None,
        tech_tags: Vec::new(),
        description_md: html_to_md(Some(&html))?,
        posted_at: item
            .released_date
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.to_utc()),
        expires_at: None,
        visa: None,
        experience: None,
        raw,
    })
}

fn level(id: &str) -> Option<&'static str> {
    match id {
        "internship" => Some("internship"),
        "entry_level" => Some("entry level"),
        "director" => Some("director"),
        "executive" => Some("executive"),
        _ => None,
    }
}
