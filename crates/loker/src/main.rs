use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use loker_core::home::{db_path, loker_home};
use loker_core::store::Store;
use loker_core::verdict::{VerdictKind, Viewer, eligible_for};

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
        /// Source id (only `himalayas` for now).
        source: String,
        /// Pages to fetch (20 jobs each).
        #[arg(long, default_value_t = 3)]
        pages: u32,
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
        /// Your UTC offset in hours, e.g. 7 or +5.5; jobs requiring other zones are dropped.
        #[arg(long, allow_hyphen_values = true)]
        tz: Option<f32>,
        #[arg(long, default_value_t = 20)]
        limit: u32,
        /// Output JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Show what is stored.
    Stats,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    let home = loker_home()?;
    let store = Store::open(&db_path(&home)).await?;

    match cli.command {
        Command::Ingest { source, pages, country } => {
            if source != loker_sources::himalayas::SOURCE_ID {
                bail!("unknown source `{source}` (available: himalayas)");
            }
            let client = loker_sources::http_client()?;
            let raw = match loker_sources::himalayas::fetch(&client, pages, country.as_deref()).await {
                Ok(raw) => raw,
                Err(e) => {
                    store.record_failure(&source, &format!("{e:#}")).await?;
                    return Err(e);
                }
            };
            let (mut jobs, mut skipped) = (Vec::new(), 0);
            for item in raw {
                match loker_sources::himalayas::transform(item) {
                    Ok(job) => jobs.push(job),
                    Err(e) => {
                        skipped += 1;
                        tracing::warn!("skipping item: {e:#}");
                    }
                }
            }
            let stats = store.ingest(&source, jobs, skipped).await?;
            println!(
                "{source}: fetched {}, new {}, updated {}, unchanged {}, skipped {}",
                stats.fetched, stats.new, stats.updated, stats.unchanged, stats.skipped
            );
        }
        Command::Search {
            query,
            country,
            tz,
            limit,
            json,
        } => {
            let viewer = country
                .map(|c| -> anyhow::Result<Viewer> {
                    let code = loker_core::country::to_alpha2(&c)
                        .with_context(|| format!("unknown country `{c}`"))?;
                    Ok(Viewer {
                        country: code.to_string(),
                        tz,
                        accept_onsite_in: Vec::new(),
                    })
                })
                .transpose()?;
            // Fetch wider than `limit` so eligibility filtering still fills the page.
            let candidates = store
                .search(&query.join(" "), limit.saturating_mul(10).max(200))
                .await?;
            let mut out = Vec::new();
            for job in candidates {
                let verdict = viewer.as_ref().map(|v| eligible_for(&job.eligibility, v));
                if matches!(verdict.as_ref().map(|v| v.kind), Some(VerdictKind::No)) {
                    continue;
                }
                out.push((job, verdict));
                if out.len() as u32 >= limit {
                    break;
                }
            }
            if json {
                let items: Vec<_> = out
                    .iter()
                    .map(|(job, verdict)| serde_json::json!({ "job": job, "eligible": verdict }))
                    .collect();
                println!("{}", serde_json::to_string_pretty(&items)?);
            } else {
                for (job, verdict) in &out {
                    let pay = job
                        .pay
                        .as_ref()
                        .map(|p| p.raw.as_str())
                        .unwrap_or("pay n/a");
                    let elig = verdict
                        .as_ref()
                        .map(|v| format!("{}: {}", format!("{:?}", v.kind).to_lowercase(), v.reason))
                        .unwrap_or_else(|| format!("{:?}", job.eligibility.scope).to_lowercase());
                    println!(
                        "{}  {} — {}\n    {elig} · {pay}\n    {}",
                        job.id, job.title, job.company, job.url
                    );
                }
                eprintln!("{} result(s)", out.len());
            }
        }
        Command::Stats => {
            for (scope, n) in store.counts().await? {
                println!("{scope:>10}  {n}");
            }
        }
    }
    Ok(())
}
