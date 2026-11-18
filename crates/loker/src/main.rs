//! CLI adapter: flags → contract params → [`App`] → text (or the same JSON MCP returns).
use std::sync::Arc;

use clap::{Parser, Subcommand};
use loker::App;
use loker_core::api::{IngestParams, SearchParams, SearchResult};
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
        /// Source id (available: himalayas).
        source: String,
        /// Pages to fetch (20 jobs each for himalayas).
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
        /// Print the contract JSON (identical to MCP structured content).
        #[arg(long)]
        json: bool,
    },
    /// Show what is stored.
    Stats {
        #[arg(long)]
        json: bool,
    },
    /// Serve the MCP tools over stdio (for Claude Code, Cursor, etc.).
    Mcp,
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
                "{source}: fetched {}, new {}, updated {}, unchanged {}, skipped {}",
                s.fetched, s.new, s.updated, s.unchanged, s.skipped
            );
        }
        Command::Search {
            query,
            country,
            tz,
            limit,
            json,
        } => {
            let q = query.join(" ");
            let result = app
                .search_jobs(SearchParams {
                    q,
                    country,
                    tz,
                    limit,
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
        println!(
            "{}  {} — {}\n    {elig} · {pay}\n    {}",
            job.id, job.title, job.company, job.url
        );
    }
    eprintln!("{} result(s)", r.count);
}
