//! CLI adapter: flags → contract params → [`App`] → text (or the same JSON MCP returns).
use std::sync::Arc;

use clap::{Parser, Subcommand};
use loker::App;
use loker_core::api::{
    CheckStatus, IngestParams, SearchParams, SearchResult, SourcesEval, VerifyParams, VerifyReport,
};
use loker_core::home::loker_home;
use serde::Serialize;

#[derive(Parser)]
#[command(
    name = "loker",
    version,
    about = "Self-hosted job aggregator: eligibility-aware search for humans and agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Fetch a source once and store what it returns.
    Ingest {
        /// Source: himalayas, jobicy, remoteok, greenhouse:<board>, ashby:<board>.
        source: String,
        /// Pages to fetch (himalayas: 20 jobs each).
        #[arg(long)]
        pages: Option<u32>,
        /// Only fetch jobs the source says are open to this country (ISO code).
        #[arg(long)]
        country: Option<String>,
    },
    /// Search stored jobs (every word must match).
    Search {
        query: Vec<String>,
        /// Only jobs open to someone based in this country (ISO code or name).
        #[arg(long)]
        country: Option<String>,
        /// Your UTC offset in hours, e.g. 7 or -5 or 5.5.
        #[arg(long, allow_hyphen_values = true)]
        tz: Option<f64>,
        #[arg(long)]
        limit: Option<u32>,
        /// Skip this many results (the previous page's "next offset").
        #[arg(long)]
        offset: Option<u32>,
        /// Print the contract JSON (identical to MCP structured content).
        #[arg(long)]
        json: bool,
    },
    /// Inspect sources.
    Sources {
        #[command(subcommand)]
        command: SourcesCommand,
    },
    /// Show what is stored.
    Stats {
        #[arg(long)]
        json: bool,
    },
    /// Fetch sources live (storing nothing) and check adapter invariants.
    /// Exits non-zero if any source fails.
    Verify {
        /// Sources to check (default: every source in the store).
        sources: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// Serve the MCP tools over stdio (for Claude Code, Cursor, etc.).
    Mcp,
}

#[derive(Subcommand)]
enum SourcesCommand {
    /// Per-source coverage, overlap, blocked merges and eligibility conflicts.
    Eval {
        #[arg(long)]
        json: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Logs go to stderr: stdout carries results, and for `mcp`, the protocol.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    let app = App::open(&loker_home()?).await?;

    match cli.command {
        Command::Ingest {
            source,
            pages,
            country,
        } => {
            let s = app
                .ingest(IngestParams {
                    source: source.clone(),
                    pages,
                    country,
                })
                .await?;
            println!(
                "{source}: fetched {}, new {}, updated {}, unchanged {}, merged {}, ambiguous {}, \
                 closed {}, reopened {}, held {}, skipped {}",
                s.fetched,
                s.new,
                s.updated,
                s.unchanged,
                s.merged,
                s.ambiguous,
                s.closed,
                s.reopened,
                s.held,
                s.skipped
            );
        }
        Command::Search {
            query,
            country,
            tz,
            limit,
            offset,
            json,
        } => {
            let q = query.join(" ");
            let result = app
                .search_jobs(SearchParams {
                    q,
                    country,
                    tz,
                    limit,
                    offset,
                })
                .await?;
            if json {
                print_json(&result)?;
            } else {
                print_search(&result);
            }
        }
        Command::Stats { json } => {
            let stats = app.stats().await?;
            if json {
                print_json(&stats)?;
            } else {
                for s in &stats.by_scope {
                    println!("{:>10}  {}", s.scope, s.count);
                }
                println!("{:>10}  {}", "total", stats.total);
            }
        }
        Command::Sources {
            command: SourcesCommand::Eval { json },
        } => {
            let eval = app.sources_eval().await?;
            if json {
                print_json(&eval)?;
            } else {
                print_eval(&eval);
            }
        }
        Command::Verify { sources, json } => {
            let report = app.verify(VerifyParams { sources }).await?;
            if json {
                print_json(&report)?;
            } else {
                print_verify(&report);
            }
            if !report.ok {
                std::process::exit(1);
            }
        }
        Command::Mcp => loker::mcp::serve_stdio(Arc::new(app)).await?,
    }
    Ok(())
}

fn print_json(v: &impl Serialize) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

fn print_search(r: &SearchResult) {
    for hit in &r.jobs {
        let job = &hit.job;
        let pay = job
            .pay
            .as_ref()
            .map(|p| p.raw.as_str())
            .unwrap_or("pay n/a");
        let elig = match &hit.eligible {
            Some(v) => format!("{}: {}", format!("{:?}", v.kind).to_lowercase(), v.reason),
            None => format!("{:?}", job.eligibility.scope).to_lowercase(),
        };
        let via: Vec<&str> = job.sources.iter().map(|s| s.source.as_str()).collect();
        println!(
            "{}  {} — {}\n    {elig} · {pay}\n    {}  (via {})",
            job.id,
            job.title,
            job.company,
            job.url,
            via.join(", ")
        );
    }
    match r.next_offset {
        Some(n) => eprintln!("{} result(s); more with --offset {n}", r.count),
        None => eprintln!("{} result(s)", r.count),
    }
}

fn print_eval(e: &SourcesEval) {
    println!(
        "{} active jobs from {} live listings ({:.2} listings per job), {} closed jobs, {} blocked merges\n",
        e.canonical_jobs, e.source_jobs, e.listings_per_job, e.closed_jobs, e.blocked_keys
    );
    println!(
        "{:<22} {:>5} {:>5} {:>6}  {:>5} {:>5} {:>5} {:>7} {:>6} {:>5}  {:>8}  last run",
        "source",
        "live",
        "gone",
        "unique",
        "scope",
        "tz",
        "pay",
        "pay_usd",
        "posted",
        "desc",
        "unmapped"
    );
    let pct = |v: f64| format!("{:.0}%", v * 100.0);
    for s in &e.sources {
        let c = &s.coverage;
        let run = s
            .last_run
            .as_ref()
            .map(|r| match &r.error {
                Some(err) => format!("{} FAILED: {err}", r.status),
                None => format!(
                    "{} new, {} merged, {} closed, {} held, {} skipped",
                    r.new.unwrap_or(0),
                    r.merged.unwrap_or(0),
                    r.closed.unwrap_or(0),
                    r.held.unwrap_or(0),
                    r.skipped.unwrap_or(0)
                ),
            })
            .unwrap_or_default();
        println!(
            "{:<22} {:>5} {:>5} {:>6}  {:>5} {:>5} {:>5} {:>7} {:>6} {:>5}  {:>8}  {run}",
            s.source,
            s.jobs,
            s.gone,
            s.unique,
            pct(c.scope),
            pct(c.timezone),
            pct(c.pay),
            pct(c.pay_usd),
            pct(c.posted_at),
            pct(c.description),
            s.location_unmapped
        );
    }
    if !e.overlaps.is_empty() {
        println!("\noverlaps");
        for o in &e.overlaps {
            println!("  {:<22} ∩ {:<22} {}", o.a, o.b, o.shared);
        }
    }
    if !e.scope_conflicts.is_empty() {
        println!("\neligibility conflicts ({})", e.scope_conflicts.len());
        for c in e.scope_conflicts.iter().take(10) {
            println!("  {}  {}\n      {}", c.job_id, c.job, c.scopes);
        }
    }
}

fn print_verify(r: &VerifyReport) {
    let mark = |s: CheckStatus| match s {
        CheckStatus::Pass => "pass",
        CheckStatus::Warn => "WARN",
        CheckStatus::Fail => "FAIL",
    };
    for src in &r.sources {
        println!(
            "{} {}  ({} items)",
            mark(src.status),
            src.source,
            src.fetched
        );
        for c in &src.checks {
            println!("    {:<4} {:<16} {}", mark(c.status), c.name, c.detail);
        }
    }
}

