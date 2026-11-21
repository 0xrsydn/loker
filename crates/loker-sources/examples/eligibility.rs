//! For `eval/eligibility`: re-reads stored jobs with the current rules and
//! prints each one's verdict for a set of viewers.
//!
//! stdin, one JSON object per line: `{"id", "viewers": [["US", -5.0], ...],
//! "listings": [{"source", "kind", "sourceJobId", "raw", "stored", "live"}]}`
//! with every listing of a canonical job as `source_jobs` holds it. Each raw
//! item goes through the adapter again (`job_from_raw`), the listings merge as
//! the store merges them (`dedup::resolve`), and the verdict is computed per
//! viewer.
//!
//! An employer board's `remote` flag isn't stored. It is recovered per
//! listing: when the stored job had a known scope and re-reading without the
//! flag gives none, the board was ingested as remote. `recovered` counts these.
//! Only employer boards take the flag, so an aggregator's listing is never
//! recovered (that would undo a rule that now reads it as unknown).
use std::io::{BufRead, Write};

use loker_core::dedup::{Linked, resolve};
use loker_core::model::{NewJob, Scope, SourceKind};
use loker_core::verdict::{Viewer, eligible_for};
use loker_sources::{Source, job_from_raw};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Listing {
    source: String,
    kind: String,
    source_job_id: String,
    raw: Value,
    stored: Option<NewJob>,
    live: bool,
}

#[derive(Deserialize)]
struct Input {
    id: String,
    viewers: Vec<(String, f64)>,
    listings: Vec<Listing>,
}

fn reread(l: &Listing) -> anyhow::Result<(NewJob, bool)> {
    let source = Source::parse(&l.source)?;
    let company = l.stored.as_ref().map(|j| j.company.as_str());
    let job = job_from_raw(&source, l.raw.clone(), company, None)?;
    let stored_known = l.kind == "employer"
        && l.stored
            .as_ref()
            .is_some_and(|j| j.eligibility.scope() != Scope::Unknown);
    if job.eligibility.scope() == Scope::Unknown && stored_known {
        return Ok((
            job_from_raw(&source, l.raw.clone(), company, Some(true))?,
            true,
        ));
    }
    Ok((job, false))
}

fn main() -> anyhow::Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in std::io::stdin().lock().lines() {
        let input: Input = serde_json::from_str(&line?)?;
        let mut linked = Vec::new();
        let mut recovered = 0;
        for l in &input.listings {
            let (job, flag) = reread(l)?;
            recovered += flag as u32;
            linked.push(Linked {
                kind: if l.kind == "employer" {
                    SourceKind::Employer
                } else {
                    SourceKind::Aggregator
                },
                source_id: l.source.clone(),
                source_job_id: l.source_job_id.clone(),
                job: Some(job),
                live: l.live,
            });
        }
        let Some(resolved) = resolve(linked) else {
            continue;
        };
        let e = resolved.job.eligibility;
        let verdicts: serde_json::Map<String, Value> = input
            .viewers
            .iter()
            .map(|(country, tz)| {
                let v = eligible_for(
                    &e,
                    &Viewer {
                        country: country.clone(),
                        tz: Some(*tz),
                        accept_onsite_in: Vec::new(),
                    },
                );
                (country.clone(), json!({"kind": v.kind, "reason": v.reason}))
            })
            .collect();
        let row =
            json!({"id": input.id, "eligibility": e, "verdicts": verdicts, "recovered": recovered});
        writeln!(out, "{row}")?;
    }
    Ok(())
}
