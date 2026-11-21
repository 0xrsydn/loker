//! Ashby job board API (public, no key):
//! `GET https://api.ashbyhq.com/posting-api/job-board/<board>?includeCompensation=true`.
//!
//! Ashby states `workplaceType` (Remote / OnSite / Hybrid) structurally, so
//! on-site jobs are recognized with high confidence. It has no company-name
//! field; the board slug is used.
use chrono::DateTime;
use loker_core::model::{Eligibility, NewJob, Pay, PayExtra, PayExtraKind, Workplace};
use loker_core::normalize::{self, eligibility_from_text, employment_type};
use serde::Deserialize;
use serde_json::Value;

use crate::{HttpClient, get_json, html_to_md};

pub async fn fetch(client: &HttpClient, board: &str) -> anyhow::Result<Vec<Value>> {
    let url = crate::url(
        "https://api.ashbyhq.com",
        &format!("/posting-api/job-board/{board}?includeCompensation=true"),
    );
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
    department: Option<String>,
    team: Option<String>,
}

#[derive(Deserialize)]
struct Secondary {
    location: Option<String>,
}

/// Ashby's pay: one tier per level (or one in all), each a list of
/// components: `Salary`, `EquityPercentage`, `EquityCashValue`, `Bonus`,
/// `Commission`, with `interval` like `1 YEAR` or `NONE`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Compensation {
    compensation_tier_summary: Option<String>,
    #[serde(default)]
    compensation_tiers: Vec<Tier>,
}

#[derive(Deserialize)]
struct Tier {
    title: Option<String>,
    #[serde(default)]
    components: Vec<Component>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Component {
    compensation_type: String,
    currency_code: Option<String>,
    interval: Option<String>,
    min_value: Option<f64>,
    max_value: Option<f64>,
}

fn pay(c: Compensation) -> Option<Pay> {
    let several = c.compensation_tiers.len() > 1;
    let (mut base, mut extras) = (Vec::new(), Vec::new());
    for tier in &c.compensation_tiers {
        for x in &tier.components {
            let period = x.interval.as_deref().and_then(normalize::pay_period);
            let money =
                || normalize::money(x.min_value, x.max_value, x.currency_code.as_deref(), period);
            let (kind, amount, percent) = match x.compensation_type.as_str() {
                "Salary" => {
                    if let Some(mut m) = money() {
                        m.label = tier.title.clone().filter(|_| several);
                        base.push(m);
                    }
                    continue;
                }
                "EquityPercentage" => (
                    PayExtraKind::Equity,
                    None,
                    normalize::percent(x.min_value, x.max_value),
                ),
                "EquityCashValue" => (PayExtraKind::Equity, money(), None),
                "Bonus" => (PayExtraKind::Bonus, money(), None),
                "Commission" => (PayExtraKind::Commission, money(), None),
                _ => continue,
            };
            extras.push(PayExtra {
                kind,
                amount,
                percent,
            });
        }
    }
    normalize::pay_from(base, extras, c.compensation_tier_summary.as_deref())
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
            workplace: Workplace::Hybrid,
            ..eligibility_from_text(&text, Some(false))
        },
        _ => eligibility_from_text(&text, item.is_remote),
    };
    let canonical = item.job_url;
    Ok(NewJob {
        source_id: source_id.into(),
        source_job_id: item.id,
        source_url: canonical.clone(),
        canonical_url: canonical,
        apply_url: item.apply_url,
        title: item.title,
        company: board.to_string(),
        seniority: None,
        category: crate::categories(
            [item.department.as_deref(), item.team.as_deref()]
                .into_iter()
                .flatten(),
        ),
        employment_type: item.employment_type.as_deref().map(employment_type),
        eligibility,
        pay: item.compensation.and_then(pay),
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
