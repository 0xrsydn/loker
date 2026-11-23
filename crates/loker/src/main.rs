use std::sync::Arc;

use clap::{Parser, Subcommand};
use loker::App;
use loker_core::api::{
    BoardTally, CheckStatus, DiscoverParams, DiscoverReport, DoctorReport, GetJobsParams,
    GetJobsResult, Hit, IngestAllReport, IngestParams, MinVerdict, RoleEnrichReport, SearchHit,
    SearchParams, SearchResult, SourcesEval, VerifyParams, VerifyReport, View,
};
use loker_core::home::loker_home;
use loker_core::model::Seniority;
use loker_core::role::{Modifier, Role};
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
    /// Fetch a source once and store what it returns (or every configured source with --all).
    Ingest {
        /// Source: himalayas, jobicy, remoteok, weworkremotely, workable, ycombinator, greenhouse:<board>, ashby:<board>, lever:<site>.
        #[arg(required_unless_present = "all")]
        source: Option<String>,
        /// Every enabled source in config.toml; exits non-zero if any fails.
        #[arg(long, conflicts_with_all = ["source", "pages", "country", "company", "remote"])]
        all: bool,
        /// Pages to fetch, default 3, at most 50 (himalayas, workable: 20 jobs each; ycombinator: 40 companies each).
        #[arg(long)]
        pages: Option<u32>,
        /// Only fetch jobs the source says are open to this country (ISO code; himalayas, workable).
        #[arg(long)]
        country: Option<String>,
        /// Company display name for an employer board (Ashby and Lever only give the slug).
        #[arg(long)]
        company: Option<String>,
        /// Every job on this employer board is remote; its locations are where you can be based.
        #[arg(long)]
        remote: bool,
        #[arg(long)]
        json: bool,
    },
    /// Search stored jobs: every word first, then partial matches.
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
        /// Rank by score alone instead of putting jobs with every word first.
        #[arg(long)]
        any_word: bool,
        /// Keep only jobs whose verdict is at least this good (needs --country).
        #[arg(long, value_parser = ["yes", "maybe"])]
        min_verdict: Option<String>,
        /// Keep only jobs in these roles or with these modifiers, at any level
        /// (`engineering`, `backend`, `forward_deployed`); see `loker roles`.
        #[arg(long, value_delimiter = ',')]
        role: Vec<String>,
        /// Keep only jobs at these levels; jobs that state none are dropped.
        #[arg(long, value_delimiter = ',', value_parser = ["intern", "junior", "mid", "senior", "lead", "executive"])]
        seniority: Vec<String>,
        /// Your years of experience: drops jobs that require more; jobs that
        /// state none are kept.
        #[arg(long)]
        years: Option<u8>,
        /// Return only id, title, company, link, scope, level, pay, years, date and verdict.
        #[arg(long)]
        compact: bool,
        /// Return only id, title, company, level and verdict.
        #[arg(long, conflicts_with = "compact")]
        titles: bool,
        /// Print the contract JSON (identical to MCP structured content).
        #[arg(long)]
        json: bool,
    },
    /// Show full details for job ids from a search.
    Get {
        ids: Vec<String>,
        /// Include the verdict for someone based in this country.
        #[arg(long)]
        country: Option<String>,
        /// Your UTC offset in hours.
        #[arg(long, allow_hyphen_values = true)]
        tz: Option<f64>,
        #[arg(long)]
        json: bool,
    },
    /// Manage $LOKER_HOME/config.toml.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
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
    /// The role vocabulary: families, roles and modifiers for --role.
    Roles {
        #[arg(long)]
        json: bool,
        #[command(subcommand)]
        command: Option<RolesCommand>,
    },
    /// Fetch sources live (storing nothing) and check adapter invariants.
    /// Exits non-zero if any source fails.
    /// Find company boards (Ashby, Greenhouse, Lever), then ask each new one's ATS whether it exists and has jobs.
    Discover {
        /// `commoncrawl` (Common Crawl's URL index), or a file of board URLs or ids (`ashby:<slug>`), one per line.
        #[arg(long, default_value = "commoncrawl")]
        from: String,
        /// Common Crawl crawl id, e.g. CC-MAIN-2026-43 (default: the newest).
        #[arg(long)]
        crawl: Option<String>,
        /// Read a crawl again although it was read before.
        #[arg(long)]
        again: bool,
        /// Check at most this many unchecked boards (default: all; 0 checks none).
        #[arg(long)]
        check: Option<u32>,
        #[arg(long)]
        json: bool,
    },
    Verify {
        /// Sources to check (default: the enabled sources in config.toml, or every source in the store without one).
        sources: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// Check the setup: home, config, database, each source's last fetch,
    /// optional provider keys. Exits non-zero if a check fails.
    Doctor {
        /// Also fetch every enabled source live (as `verify`).
        #[arg(long)]
        live: bool,
        #[arg(long)]
        json: bool,
    },
    /// Print the v1 contract's JSON Schemas, or write them to a directory.
    Schema {
        #[arg(long)]
        out: Option<std::path::PathBuf>,
    },
    /// Serve the MCP tools over stdio (for Claude Code, Cursor, etc.).
    Mcp,
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Write a starting config.toml (refuses to overwrite one).
    Init {
        /// Where you are (ISO code): turns on Himalayas' and Workable's search for it.
        #[arg(long)]
        country: Option<String>,
    },
    /// Parse config.toml and list the sources it enables.
    Check,
}

#[derive(Subcommand)]
enum RolesCommand {
    /// Classify roles with the model in [providers.jev] (needs
    /// OPENROUTER_API_KEY): jobs with no cached answer, then every job again.
    Enrich {
        /// Ask about at most this many jobs.
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long)]
        json: bool,
    },
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
    if let Command::Schema { out } = &cli.command {
        return write_schemas(out.as_deref());
    }
    let app = App::open(&loker_home()?).await?;

    match cli.command {
        Command::Ingest {
            all: true, json, ..
        } => {
            let report = app.ingest_all().await?;
            if let Some(n) = &report.notice {
                eprintln!("{n}");
            }
            if json {
                print_json(&report)?;
            } else {
                print_ingest_all(&report);
            }
            if !report.ok {
                std::process::exit(1);
            }
        }
        Command::Ingest {
            source,
            pages,
            country,
            company,
            remote,
            json,
            ..
        } => {
            let source = source.expect("clap requires a source without --all");
            let s = app
                .ingest(IngestParams {
                    source: source.clone(),
                    pages,
                    country,
                    company,
                    remote: remote.then_some(true),
                })
                .await?;
            if json {
                print_json(&s)?;
            } else {
                println!("{source}: {}", ingest_line(&s));
            }
        }
        Command::Config {
            command: ConfigCommand::Init { country },
        } => {
            let path = loker_core::config::init(app.home(), country.as_deref())?;
            println!("wrote {}", path.display());
            println!("next: loker ingest --all, then loker doctor");
        }
        Command::Config {
            command: ConfigCommand::Check,
        } => {
            let path = loker_core::config::path(app.home());
            let sources = app
                .config_check()?
                .ok_or_else(|| anyhow::anyhow!("no {} (`loker config init`)", path.display()))?;
            let mut bad = 0;
            for s in &sources {
                let state = if s.enabled { "enabled " } else { "disabled" };
                match &s.error {
                    None => println!("{state} {}", s.id),
                    Some(e) => {
                        bad += 1;
                        println!("INVALID  {}: {e}", s.id)
                    }
                }
            }
            if bad > 0 {
                anyhow::bail!("{bad} invalid source(s) in {}", path.display());
            }
            println!("{} ok", path.display());
        }
        Command::Search {
            query,
            country,
            tz,
            limit,
            offset,
            min_verdict,
            role,
            seniority,
            years,
            compact,
            titles,
            any_word,
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
                    min_verdict: min_verdict.map(|v| match v.as_str() {
                        "yes" => MinVerdict::Yes,
                        _ => MinVerdict::Maybe,
                    }),
                    role: role
                        .iter()
                        .map(|r| {
                            serde_json::from_value(serde_json::Value::String(r.clone())).map_err(
                                |_| anyhow::anyhow!("unknown role `{r}` (see `loker roles`)"),
                            )
                        })
                        .collect::<anyhow::Result<_>>()?,
                    seniority: seniority
                        .iter()
                        .map(|s| serde_json::from_value(serde_json::Value::String(s.clone())))
                        .collect::<Result<_, _>>()?,
                    experience_years: years,
                    view: if titles {
                        View::Titles
                    } else if compact {
                        View::Compact
                    } else {
                        View::Full
                    },
                    any_word,
                })
                .await?;
            if json {
                print_json(&result)?;
            } else {
                print_search(&result);
            }
        }
        Command::Get {
            ids,
            country,
            tz,
            json,
        } => {
            let result = app.get_jobs(GetJobsParams { ids, country, tz }).await?;
            if json {
                print_json(&result)?;
            } else {
                print_get(&result);
            }
        }
        Command::Roles {
            command: Some(RolesCommand::Enrich { limit, json }),
            ..
        } => {
            let report = app.enrich_roles(limit).await?;
            if json {
                print_json(&report)?;
            } else {
                print_enrich(&report);
            }
            if report.failed > 0 {
                std::process::exit(1);
            }
        }
        Command::Roles { json, .. } => {
            let roles = app.list_roles();
            if json {
                print_json(&roles)?;
            } else {
                for r in &roles.roles {
                    let id = id_str(r.id);
                    let indent = if r.family.is_some() { "  " } else { "" };
                    println!("{indent}{id:<24} {}", r.definition);
                }
                println!();
                for m in &roles.modifiers {
                    println!("{:<24} {} (modifier)", id_str(m.id), m.definition);
                }
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
        Command::Discover {
            from,
            crawl,
            again,
            check,
            json,
        } => {
            let report = app
                .discover(DiscoverParams {
                    from,
                    crawl,
                    again,
                    check,
                })
                .await?;
            if json {
                print_json(&report)?;
            } else {
                print_discover(&report);
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
        Command::Doctor { live, json } => {
            let report = app.doctor(live).await?;
            if json {
                print_json(&report)?;
            } else {
                print_doctor(&report);
            }
            if !report.ok {
                std::process::exit(1);
            }
        }
        Command::Schema { .. } => unreachable!("handled before opening the store"),
        Command::Mcp => loker::mcp::serve_stdio(Arc::new(app)).await?,
    }
    Ok(())
}

fn print_enrich(r: &RoleEnrichReport) {
    println!(
        "{}: asked {}, answered {}, failed {}, ${:.4}; {} job(s) changed role",
        r.model, r.asked, r.answered, r.failed, r.cost_usd, r.reclassified
    );
    for e in &r.errors {
        eprintln!("  {e}");
    }
}

fn print_json(v: &impl Serialize) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

fn print_search(r: &SearchResult) {
    if let Some(n) = &r.notice {
        println!("{n}");
    }
    for hit in &r.jobs {
        match hit {
            Hit::Full(hit) => print_hit(hit),
            Hit::Compact(c) => {
                let elig = match &c.eligible {
                    Some(v) => format!("{}: {}", format!("{:?}", v.kind).to_lowercase(), v.reason),
                    None => format!("{:?}", c.scope).to_lowercase(),
                };
                let pay = c.pay.as_deref().unwrap_or("pay n/a");
                let yrs = years_text(c.min_years);
                println!(
                    "{}  {} — {}{}{}\n    {elig} · {pay}{yrs}\n    {}",
                    c.id,
                    c.title,
                    c.company,
                    about(c.seniority, &c.roles, &c.modifiers),
                    partial_mark(c.partial),
                    c.url
                );
            }
            Hit::Titles(t) => {
                let verdict = t
                    .verdict
                    .map(|v| format!(" · {}", format!("{v:?}").to_lowercase()))
                    .unwrap_or_default();
                println!(
                    "{}  {} — {}{}{verdict}{}",
                    t.id,
                    t.title,
                    t.company,
                    about(t.seniority, &t.roles, &t.modifiers),
                    partial_mark(t.partial)
                );
            }
        }
    }
    match r.next_offset {
        Some(n) => eprintln!(
            "{} of {} result(s), {} with every word; more with --offset {n}",
            r.count, r.total, r.full_matches
        ),
        None => eprintln!(
            "{} of {} result(s), {} with every word",
            r.count, r.total, r.full_matches
        ),
    }
}

fn print_hit(hit: &SearchHit) {
    let job = &hit.job;
    let pay = job
        .pay
        .as_ref()
        .map(|p| p.raw.as_str())
        .unwrap_or("pay n/a");
    let elig = match &hit.eligible {
        Some(v) => format!("{}: {}", format!("{:?}", v.kind).to_lowercase(), v.reason),
        None => format!("{:?}", job.eligibility.scope()).to_lowercase(),
    };
    let yrs = years_text(job.experience.as_ref().map(|e| e.min_years));
    let via: Vec<&str> = job.sources.iter().map(|s| s.source.as_str()).collect();
    let roles: Vec<Role> = job
        .role
        .iter()
        .flat_map(|r| std::iter::once(r.primary).chain(r.secondary.iter().copied()))
        .collect();
    let modifiers = job
        .role
        .as_ref()
        .map(|r| r.modifiers.as_slice())
        .unwrap_or(&[]);
    println!(
        "{}  {} — {}{}{}\n    {elig} · {pay}{yrs}\n    {}  (via {})",
        job.id,
        job.title,
        job.company,
        about(job.seniority, &roles, modifiers),
        partial_mark(hit.partial),
        job.url,
        via.join(", ")
    );
}

fn id_str(v: impl Serialize) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn roles_text(roles: &[Role], modifiers: &[Modifier]) -> String {
    let ids: Vec<String> = roles
        .iter()
        .map(id_str)
        .chain(modifiers.iter().map(id_str))
        .collect();
    if ids.is_empty() {
        String::new()
    } else {
        format!(" [{}]", ids.join(", "))
    }
}

fn about(s: Option<Seniority>, roles: &[Role], modifiers: &[Modifier]) -> String {
    format!("{}{}", level(s), roles_text(roles, modifiers))
}

fn level(s: Option<Seniority>) -> String {
    s.map(|s| {
        format!(
            " · {}",
            serde_json::to_value(s).unwrap().as_str().unwrap_or("")
        )
    })
    .unwrap_or_default()
}

fn years_text(min: Option<u8>) -> String {
    match min {
        Some(0) => " · no experience needed".into(),
        Some(n) => format!(" · {n}+ yrs"),
        None => String::new(),
    }
}

fn partial_mark(partial: bool) -> &'static str {
    if partial { "  (partial match)" } else { "" }
}

fn print_get(r: &GetJobsResult) {
    for hit in &r.jobs {
        print_hit(hit);
        if hit.job.status != "active" {
            println!("    status: {}", hit.job.status);
        }
    }
    for id in &r.missing {
        eprintln!("{id}: no such job");
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

fn print_discover(r: &DiscoverReport) {
    let tally = |t: &BoardTally| {
        format!(
            "{} live, {} empty, {} missing, {} private, {} unchecked, {} failed",
            t.live, t.empty, t.missing, t.private, t.unchecked, t.failed
        )
    };
    if r.skipped {
        println!("{}: read before (--again reads it anew)", r.finder);
    } else {
        println!(
            "{}: {} boards, {} new, {} unreadable",
            r.finder, r.found, r.new, r.unreadable
        );
    }
    println!("checked: {}", tally(&r.checked));
    println!("boards:  {}", tally(&r.boards));
    for e in &r.errors {
        println!("    {e}");
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

fn print_doctor(r: &DoctorReport) {
    let mark = |s: CheckStatus| match s {
        CheckStatus::Pass => "pass",
        CheckStatus::Warn => "WARN",
        CheckStatus::Fail => "FAIL",
    };
    for c in &r.checks {
        println!("{:<4} {:<28} {}", mark(c.status), c.name, c.detail);
        if let Some(fix) = &c.fix {
            println!("     {:<28} -> {fix}", "");
        }
    }
    let n = |s| r.checks.iter().filter(|c| c.status == s).count();
    println!(
        "{} pass, {} warn, {} fail",
        n(CheckStatus::Pass),
        n(CheckStatus::Warn),
        n(CheckStatus::Fail)
    );
}

fn write_schemas(out: Option<&std::path::Path>) -> anyhow::Result<()> {
    let schemas = loker::contract_schemas();
    match out {
        Some(dir) => {
            std::fs::create_dir_all(dir)?;
            for (name, schema) in &schemas {
                std::fs::write(
                    dir.join(name),
                    format!("{}\n", serde_json::to_string_pretty(schema)?),
                )?;
            }
            eprintln!("wrote {} schemas to {}", schemas.len(), dir.display());
        }
        None => {
            let all: serde_json::Map<String, serde_json::Value> = schemas
                .into_iter()
                .map(|(n, s)| (n.to_string(), s))
                .collect();
            print_json(&all)?;
        }
    }
    Ok(())
}

fn ingest_line(s: &loker_core::model::IngestStats) -> String {
    format!(
        "fetched {}, new {}, updated {}, unchanged {}, merged {}, ambiguous {}, \
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
    )
}

fn print_ingest_all(r: &IngestAllReport) {
    for o in &r.sources {
        match (&o.stats, &o.error) {
            (Some(s), _) => println!("{}: {}", o.source, ingest_line(s)),
            (None, Some(e)) => println!("{}: FAILED: {e}", o.source),
            (None, None) => {}
        }
    }
    if let Some(roles) = &r.roles {
        print!("roles: ");
        print_enrich(roles);
    }
}
