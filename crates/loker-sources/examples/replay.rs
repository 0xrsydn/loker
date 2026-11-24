use std::collections::BTreeSet;
use std::io::BufRead;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use loker_core::store::{Batch, SourceMeta, Store};
use loker_sources::{Source, job_from_raw};
use serde_json::Value;

struct Group {
    source: String,
    company: Option<String>,
    raw: Vec<Value>,
}

fn groups() -> anyhow::Result<Vec<Group>> {
    let mut out: Vec<Group> = Vec::new();
    for line in std::io::stdin().lock().lines() {
        let mut v: Value = serde_json::from_str(&line?)?;
        let source = v["source"].as_str().unwrap_or_default().to_string();
        let company = v["company"].as_str().map(str::to_string);
        let raw = v["raw"].take();
        match out.last_mut() {
            Some(g) if g.source == source => g.raw.push(raw),
            _ => out.push(Group {
                source,
                company,
                raw: vec![raw],
            }),
        }
    }
    Ok(out)
}

async fn replay(store: &Store, groups: &[Group]) -> anyhow::Result<(Duration, Duration, usize)> {
    let (mut read, mut write, mut jobs) = (Duration::ZERO, Duration::ZERO, 0);
    for g in groups {
        let source = Source::parse(&g.source)?;
        let start = Instant::now();
        let complete_ids: Option<BTreeSet<String>> = g
            .raw
            .iter()
            .map(|item| source.complete_id(item))
            .collect::<Option<Vec<Option<String>>>>()
            .and_then(|ids| ids.into_iter().collect());
        let mut batch = Vec::with_capacity(g.raw.len());
        let mut skipped = 0;
        for item in &g.raw {
            match job_from_raw(&source, item.clone(), g.company.as_deref(), None) {
                Ok(job) => batch.push(job),
                Err(_) => skipped += 1,
            }
        }
        read += start.elapsed();
        jobs += batch.len();
        let start = Instant::now();
        store
            .ingest(Batch {
                source: SourceMeta {
                    id: source.id(),
                    kind: source.kind(),
                    dated_by_age: source.dated_by_age(),
                },
                jobs: batch,
                skipped,
                complete_ids,
                now: chrono::Utc::now(),
            })
            .await?;
        write += start.elapsed();
    }
    Ok((read, write, jobs))
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .init();
    let db = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: replay OUT.db < items.jsonl"),
    );
    let passes: usize = std::env::args().nth(2).map_or(Ok(2), |n| n.parse())?;
    let groups = groups()?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let start = Instant::now();
        let store = Store::open(&db).await?;
        println!("open {:.2}s", start.elapsed().as_secs_f64());
        for pass in 1..=passes {
            let start = Instant::now();
            let (read, write, jobs) = replay(&store, &groups).await?;
            println!(
                "pass {pass}: {jobs} jobs from {} sources in {:.2}s (transform {:.2}s, store {:.2}s)",
                groups.len(),
                start.elapsed().as_secs_f64(),
                read.as_secs_f64(),
                write.as_secs_f64()
            );
        }
        anyhow::Ok(())
    })
}
