//! We Work Remotely RSS feeds (public, no key): the all-jobs feed and one
//! feed per category, merged by guid. The all-jobs feed holds only about ten
//! of each category's newest; the category feeds go deeper. A category feed
//! that fails is skipped (its slug may have changed); the all-jobs feed must
//! load. Not a complete list, so missing items close nothing: `expires_at`
//! and staleness close them.
//!
//! Each item states `region` ("Anywhere in the World", "USA Only", "Europe
//! Only", ...), which is who may apply, and its title is "Company: Role".
//! Some also state `country` ("🇨🇦 Canada and 🇺🇸 United States of
//! America"): the countries the hire may be in, narrower than a region that
//! often says "Anywhere in the World" beside it.
use std::collections::BTreeSet;

use chrono::DateTime;
use loker_core::model::{Eligibility, Elsewhere, NewJob};
use loker_core::normalize::{eligibility_from_text, employment_type};
use serde_json::{Map, Value};

use crate::{HttpClient, get_text, html_to_md};

pub const SOURCE_ID: &str = "weworkremotely";

const ALL: &str = "/remote-jobs.rss";
const CATEGORIES: &[&str] = &[
    "remote-full-stack-programming-jobs",
    "remote-back-end-programming-jobs",
    "remote-front-end-programming-jobs",
    "remote-devops-sysadmin-jobs",
    "remote-design-jobs",
    "remote-product-jobs",
    "remote-customer-support-jobs",
    "remote-sales-and-marketing-jobs",
    "remote-management-and-finance-jobs",
    "all-other-remote-jobs",
];

pub async fn fetch(client: &HttpClient) -> anyhow::Result<Vec<Value>> {
    let base = "https://weworkremotely.com";
    let mut items = parse(&get_text(client, &crate::url(base, ALL)).await?)?;
    for slug in CATEGORIES {
        let path = format!("/categories/{slug}.rss");
        match get_text(client, &crate::url(base, &path)).await {
            Ok(xml) => items.extend(parse(&xml)?),
            Err(e) => tracing::warn!(%path, "skipping a category feed: {e:#}"),
        }
    }
    let mut seen = BTreeSet::new();
    items.retain(|i| seen.insert(i.get("guid").and_then(Value::as_str).map(str::to_owned)));
    Ok(items)
}

/// Each `<item>` as a JSON object of its child elements' text.
fn parse(xml: &str) -> anyhow::Result<Vec<Value>> {
    let doc = roxmltree::Document::parse(xml)?;
    Ok(doc
        .descendants()
        .filter(|n| n.has_tag_name("item"))
        .map(|item| {
            let fields: Map<String, Value> = item
                .children()
                .filter(|c| c.is_element() && c.tag_name().namespace().is_none())
                .filter_map(|c| {
                    let text = c.text()?.trim();
                    (!text.is_empty()).then(|| (c.tag_name().name().to_string(), Value::from(text)))
                })
                .collect();
            Value::Object(fields)
        })
        .collect())
}

/// "🇫🇷 France, 🇩🇪 Germany, and 🇬🇧 United Kingdom of Great Britain and
/// Northern Ireland" → "France; Germany; United Kingdom of Great Britain and
/// Northern Ireland". Each country follows its flag, so names that hold
/// "and" stay whole.
fn countries(field: &str) -> String {
    let flag = |c: char| ('\u{1F1E6}'..='\u{1F1FF}').contains(&c);
    field
        .split(flag)
        .map(|p| {
            p.trim()
                .trim_end_matches(',')
                .trim_end_matches(", and")
                .trim_end_matches(" and")
                .trim_end_matches(',')
                .trim()
        })
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn transform(raw: Value) -> anyhow::Result<NewJob> {
    let s = |k: &str| raw.get(k).and_then(Value::as_str).map(str::to_owned);
    let link = s("link").ok_or_else(|| anyhow::anyhow!("missing link"))?;
    let heading = s("title").ok_or_else(|| anyhow::anyhow!("missing title"))?;
    let (company, title) = heading
        .split_once(": ")
        .ok_or_else(|| anyhow::anyhow!("title is not \"Company: Role\": {heading}"))?;
    let id = link.rsplit('/').next().unwrap_or(&link).to_string();
    let region = s("region").unwrap_or_default();
    let date = |k: &str| {
        s(k).and_then(|d| DateTime::parse_from_rfc2822(&d).ok())
            .map(|t| t.to_utc())
    };
    Ok(NewJob {
        source_id: SOURCE_ID.into(),
        source_job_id: id,
        source_url: link.clone(),
        canonical_url: link.clone(),
        apply_url: Some(link),
        title: title.to_string(),
        company: company.to_string(),
        seniority: None,
        category: s("category"),
        employment_type: s("type").as_deref().map(employment_type),
        eligibility: match s("country")
            .map(|c| countries(&c))
            .filter(|c| !c.is_empty())
        {
            Some(places) => eligibility_from_text(&places, Some(true)),
            // The board's default, and no evidence on its own: most postings
            // that carry it without `country` restrict who may apply in their
            // text (eval/eligibility: 2 of 11 open to all five viewers). It
            // says remote, not where.
            None if region == "Anywhere in the World" => Eligibility {
                raw: Some(region.clone()),
                elsewhere: Elsewhere::NotStated,
                evidence: Some(
                    "\"Anywhere in the World\" is the board's default; the posting may say where"
                        .into(),
                ),
                ..eligibility_from_text("", Some(true))
            },
            None => eligibility_from_text(region.trim_end_matches(" Only"), Some(true)),
        },
        pay: None,
        tech_tags: s("skills")
            .map(|k| {
                k.split([',', ';'])
                    .flat_map(|p| p.split(" and "))
                    .map(|t| t.trim().to_lowercase())
                    .filter(|t| !t.is_empty())
                    .collect()
            })
            .unwrap_or_default(),
        description_md: html_to_md(s("description").as_deref())?,
        posted_at: date("pubDate"),
        expires_at: date("expires_at"),
        visa: None,
        raw,
    })
}
