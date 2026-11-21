//! Dedup rules, with no IO: which listings are one job, and what that job says.
//!
//! The store loads rows, asks these functions, and writes the answer back, so
//! every rule here can be read (and tested) without a database:
//!
//! * [`cluster`]: given the live listings that share a match key, keep them
//!   apart, merge them, or split them because the key is ambiguous.
//! * [`resolve`]: given every listing linked to one canonical job, which of
//!   them count, in what precedence, the merged job and whether it is active.
use std::collections::BTreeSet;

use crate::model::{NewJob, SourceKind};

/// A live listing under one match key, as the store found it.
#[derive(Debug, Clone)]
pub struct Member {
    pub source_id: String,
    pub source_job_id: String,
    pub canonical_id: String,
}

/// What to do with the listings under one key. Indexes point into the
/// `members` given to [`cluster`].
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    /// Nothing to do: one listing, an empty key, or one source repeating a
    /// title (nothing to merge, and nothing was blocked).
    Keep,
    /// One job: move these members onto canonical job `into`.
    Merge { into: String, moves: Vec<usize> },
    /// The key can't tell the jobs apart: each of these members leaves for a
    /// canonical job of its own.
    Split { evict: Vec<usize> },
}

impl Plan {
    /// A merge was blocked because some source repeats the key.
    pub fn is_ambiguous(&self) -> bool {
        matches!(self, Plan::Split { .. })
    }
}

/// Re-cluster the live listings sharing `key` (company_key, title_key).
///
/// `members` must be ordered oldest canonical job first; ties are broken by
/// source and source job id, so the outcome depends only on the stored rows.
///
/// If each source contributes at most one listing, they are one job and all
/// move onto the oldest canonical job. If any source has two (one title posted
/// in several cities, say), every listing stays on a canonical job of its own:
/// the first member on each canonical job keeps it, the rest are evicted.
pub fn cluster(key: &(String, String), members: &[Member]) -> Plan {
    if members.len() <= 1 || key.0.is_empty() || key.1.is_empty() {
        return Plan::Keep;
    }
    let sources: BTreeSet<&str> = members.iter().map(|m| m.source_id.as_str()).collect();
    if sources.len() == 1 {
        return Plan::Keep;
    }
    if sources.len() == members.len() {
        let into = members[0].canonical_id.clone();
        let moves = (0..members.len())
            .filter(|&i| members[i].canonical_id != into)
            .collect();
        Plan::Merge { into, moves }
    } else {
        let mut seen = BTreeSet::new();
        let evict = (0..members.len())
            .filter(|&i| !seen.insert(members[i].canonical_id.as_str()))
            .collect();
        Plan::Split { evict }
    }
}

/// A listing linked to a canonical job.
#[derive(Debug, Clone)]
pub struct Linked {
    pub kind: SourceKind,
    pub source_id: String,
    pub source_job_id: String,
    /// The normalized job; `None` for rows stored before it was kept.
    pub job: Option<NewJob>,
    pub live: bool,
}

/// A canonical job as its listings describe it.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub job: NewJob,
    /// At least one listing is live.
    pub active: bool,
}

/// The canonical job its listings describe: fields from the live listings
/// (from all of them once none is live), active exactly while one is live.
/// `None` when no listing carries a stored job yet.
pub fn resolve(linked: Vec<Linked>) -> Option<Resolved> {
    let active = linked.iter().any(|l| l.live);
    let mut members: Vec<_> = linked
        .into_iter()
        .filter(|l| l.live || !active)
        .filter_map(|l| Some(((l.kind, l.source_id, l.source_job_id), l.job?)))
        .collect();
    if members.is_empty() {
        return None;
    }
    // Precedence: employer boards first, then by source id.
    members.sort_by(|a, b| a.0.cmp(&b.0));
    let jobs: Vec<NewJob> = members.into_iter().map(|(_, j)| j).collect();
    Some(Resolved {
        job: merge(&jobs),
        active,
    })
}

/// Merge jobs (in precedence order) into one. The first job supplies identity
/// and text; eligibility comes from a source that names places when one does
/// ("Canada" over another board's "Anywhere": the narrower statement is the
/// rule), then from whichever is most confident; pay prefers a source that
/// states base pay.
fn names_places(e: &crate::model::Eligibility) -> bool {
    !e.countries.is_empty() || !e.regions.is_empty() || e.onsite_city.is_some()
}

fn merge(jobs: &[NewJob]) -> NewJob {
    let primary = &jobs[0];
    let first = |f: fn(&NewJob) -> Option<String>| jobs.iter().find_map(f);
    let eligibility = jobs
        .iter()
        .map(|j| &j.eligibility)
        .fold(&primary.eligibility, |best, e| {
            if (names_places(e), e.confidence) > (names_places(best), best.confidence) {
                e
            } else {
                best
            }
        })
        .clone();
    let pay = jobs
        .iter()
        .filter_map(|j| j.pay.as_ref())
        .find(|p| !p.base.is_empty())
        .or_else(|| jobs.iter().find_map(|j| j.pay.as_ref()))
        .cloned();
    let mut tech_tags: Vec<String> = jobs
        .iter()
        .flat_map(|j| j.tech_tags.iter().cloned())
        .collect();
    tech_tags.sort();
    tech_tags.dedup();
    NewJob {
        source_id: primary.source_id.clone(),
        source_job_id: primary.source_job_id.clone(),
        source_url: primary.source_url.clone(),
        canonical_url: primary.canonical_url.clone(),
        apply_url: primary
            .apply_url
            .clone()
            .or_else(|| Some(primary.source_url.clone())),
        title: primary.title.clone(),
        company: primary.company.clone(),
        seniority: first(|j| j.seniority.clone()),
        employment_type: first(|j| j.employment_type.clone()),
        // Every listing's category: each is evidence of what the job is.
        category: {
            let mut parts: Vec<&str> = Vec::new();
            for c in jobs.iter().filter_map(|j| j.category.as_deref()) {
                for p in c.split("; ") {
                    if !parts.contains(&p) {
                        parts.push(p);
                    }
                }
            }
            (!parts.is_empty()).then(|| parts.join("; "))
        },
        eligibility,
        pay,
        tech_tags,
        description_md: first(|j| j.description_md.clone().filter(|d| !d.trim().is_empty())),
        posted_at: jobs.iter().filter_map(|j| j.posted_at).min(),
        expires_at: jobs.iter().filter_map(|j| j.expires_at).max(),
        // The first source that states sponsorship at all, employer boards
        // first (they sort ahead).
        visa: jobs.iter().find_map(|j| j.visa.clone()),
        experience: jobs.iter().find_map(|j| j.experience.clone()),
        raw: serde_json::Value::Null,
    }
}
