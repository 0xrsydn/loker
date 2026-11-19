//! End to end, offline: the real `loker` binary against a local stub that
//! serves one small hand-written payload per adapter shape (not snapshots).
//!
//! Covers ingest through every adapter, idempotent re-runs, order-independent
//! dedup, closing and reopening through a real complete fetch, CLI = MCP
//! parity, `loker verify` (pass and fail), and drift of the committed v1
//! contract schemas.
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

// ---------- stub sources ----------

/// Serves every adapter's endpoint on one port. `shrink` drops a job from the
/// Ashby board, as if the company closed it.
struct Stub {
    base: String,
    shrink: Arc<AtomicBool>,
    /// Every request target served, in order.
    log: Arc<Mutex<Vec<String>>>,
}

fn stub() -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let shrink = Arc::new(AtomicBool::new(false));
    let flag = shrink.clone();
    let log = Arc::new(Mutex::new(Vec::new()));
    let served = log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                head.push(byte[0]);
            }
            let line = String::from_utf8_lossy(&head);
            let target = line.split_whitespace().nth(1).unwrap_or("/").to_string();
            served.lock().unwrap().push(target.clone());
            let (status, body) = route(&target, flag.load(Ordering::SeqCst));
            let body = body.to_string();
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    Stub { base, shrink, log }
}

fn route(target: &str, shrink: bool) -> (&'static str, Value) {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    match path {
        "/jobs/api/search" if query.contains("page=1") => {
            ("200 OK", json!({ "jobs": himalayas() }))
        }
        "/jobs/api/search" => ("200 OK", json!({ "jobs": [] })),
        "/jobs/api" => ("200 OK", json!({ "jobs": himalayas(), "nextCursor": null })),
        "/v1/boards/acme/jobs" => ("200 OK", json!({ "jobs": greenhouse() })),
        "/v1/boards/broken/jobs" => ("500 Internal Server Error", json!({ "error": "boom" })),
        "/posting-api/job-board/globex" => {
            let mut jobs = ashby();
            if shrink {
                jobs.pop();
            }
            ("200 OK", json!({ "apiVersion": "1", "jobs": jobs }))
        }
        "/api/v2/remote-jobs" => ("200 OK", json!({ "jobs": jobicy() })),
        // Paged: the second page is empty only if the adapter asks for one.
        "/v0/postings/initech" if query.contains("skip=0") => ("200 OK", Value::Array(lever())),
        "/v0/postings/initech" => ("200 OK", json!([])),
        "/api" => {
            let mut items = vec![json!({ "legal": "API terms: link back to Remote OK." })];
            items.extend(remoteok());
            ("200 OK", Value::Array(items))
        }
        _ => ("404 Not Found", json!({})),
    }
}

/// Far in the future, so nothing expires during the test.
const LATER: i64 = 4_102_444_800;

fn himalayas() -> Vec<Value> {
    vec![
        json!({
            "guid": "https://himalayas.app/companies/acme/jobs/backend-engineer",
            "title": "Backend Engineer", "companyName": "Acme",
            "applicationLink": "https://himalayas.app/companies/acme/jobs/backend-engineer?utm_source=api",
            "locationRestrictions": [], "timezoneRestrictions": [],
            "minSalary": 120000, "maxSalary": 150000, "currency": "USD", "salaryPeriod": "annual",
            "seniority": ["Senior"], "employmentType": "Full Time",
            "pubDate": 1_790_000_000, "expiryDate": LATER,
            "description": "<p>Build <b>APIs</b>.</p>"
        }),
        json!({
            "guid": "https://himalayas.app/companies/other/jobs/designer",
            // Messy on purpose: the shared cleanup (normalize::tidy) must fix it.
            "title": "  Designer &amp;\n Illustrator ", "companyName": "Other ",
            "applicationLink": "https://HIMALAYAS.app/companies/other/jobs/designer?utm_source=api&ref=feed",
            "locationRestrictions": ["Indonesia", "Vietnam"], "timezoneRestrictions": [7, 8],
            "seniority": [], "employmentType": "Contractor",
            "pubDate": 1_790_000_100, "expiryDate": LATER,
            "description": "<p>Design things.</p>"
        }),
    ]
}

fn greenhouse() -> Vec<Value> {
    vec![
        json!({
            "id": 101, "title": "Backend Engineer", "company_name": "Acme, Inc.",
            "absolute_url": "https://job-boards.greenhouse.io/acme/jobs/101?gh_src=x",
            "location": { "name": "Remote" },
            "first_published": "2026-09-20T10:00:00-04:00",
            "content": "&lt;p&gt;Build APIs at Acme.&lt;/p&gt;"
        }),
        json!({
            // Greenhouse doesn't say remote or on-site; this lists countries only.
            "id": 102, "title": "Data Analyst", "company_name": "Acme, Inc.",
            "absolute_url": "https://job-boards.greenhouse.io/acme/jobs/102",
            "location": { "name": "Germany; Netherlands" },
            "first_published": "2026-09-21T10:00:00-04:00",
            "content": "&lt;p&gt;Numbers.&lt;/p&gt;"
        }),
    ]
}

fn ashby() -> Vec<Value> {
    vec![
        json!({
            "id": "a-1", "title": "Frontend Engineer", "location": "Remote",
            "secondaryLocations": [], "isRemote": true, "workplaceType": "Remote", "isListed": true,
            "jobUrl": "https://jobs.ashbyhq.com/globex/a-1",
            "applyUrl": "https://jobs.ashbyhq.com/globex/a-1/application",
            "publishedAt": "2026-09-22T10:00:00.000+00:00", "employmentType": "FullTime",
            "descriptionHtml": "<p>React.</p>",
            "compensation": { "compensationTierSummary": "$100K – $130K" }
        }),
        json!({
            "id": "a-2", "title": "Support Lead", "location": "Berlin",
            "secondaryLocations": [], "isRemote": false, "workplaceType": "OnSite", "isListed": true,
            "jobUrl": "https://jobs.ashbyhq.com/globex/a-2",
            "applyUrl": "https://jobs.ashbyhq.com/globex/a-2/application",
            "publishedAt": "2026-09-23T10:00:00.000+00:00", "employmentType": "FullTime",
            "descriptionHtml": "<p>Help customers.</p>"
        }),
    ]
}

fn jobicy() -> Vec<Value> {
    vec![json!({
        "id": 9001, "url": "https://jobicy.com/jobs/9001-writer", "jobTitle": "Writer",
        "companyName": "Initech", "jobGeo": "Anywhere", "jobType": "['Full-Time']", "jobLevel": "Any",
        "salaryMin": 50000, "salaryMax": 60000, "salaryCurrency": "USD", "salaryPeriod": "yearly",
        "pubDate": "2026-09-24T10:00:00+00:00", "jobDescription": "<p>Words.</p>"
    })]
}

fn lever() -> Vec<Value> {
    vec![
        // The same role as Jobicy's: one job across two sources.
        json!({
            "id": "lv-1", "text": "Writer", "workplaceType": "remote", "country": "US",
            "categories": { "commitment": "Full-time", "location": "Anywhere", "allLocations": ["Anywhere"] },
            "hostedUrl": "https://jobs.lever.co/initech/lv-1",
            "applyUrl": "https://jobs.lever.co/initech/lv-1/apply",
            "createdAt": 1_790_000_000_000_i64, "description": "<p>Words.</p>",
            "lists": [{ "text": "You have", "content": "<li>A pen</li>" }], "additional": ""
        }),
        // On-site, with Lever's ISO country and a yearly salary.
        json!({
            "id": "lv-2", "text": "Office Manager", "workplaceType": "onsite", "country": "US",
            "categories": { "commitment": "Full-time", "location": "New York, NY", "allLocations": ["New York, NY"] },
            "hostedUrl": "https://jobs.lever.co/initech/lv-2",
            "applyUrl": "https://jobs.lever.co/initech/lv-2/apply",
            "createdAt": 1_790_000_100_000_i64, "description": "<p>Run the office.</p>",
            "lists": [], "additional": "",
            "salaryRange": { "currency": "USD", "interval": "per-year-salary", "min": 60000, "max": 80000 }
        }),
    ]
}

fn remoteok() -> Vec<Value> {
    vec![json!({
        "id": "777", "slug": "remote-frontend-engineer-globex-777", "position": "Frontend Engineer",
        "company": "Globex", "location": "Worldwide", "tags": ["react", "typescript"],
        "url": "https://remoteOK.com/remote-jobs/remote-frontend-engineer-globex-777",
        "apply_url": "https://remoteOK.com/remote-jobs/remote-frontend-engineer-globex-777",
        "salary_min": 90000, "salary_max": 110000, "date": "2026-09-25T10:00:00+00:00",
        "description": "<p>React at Globex.</p>"
    })]
}

// ---------- running loker ----------

struct Home {
    dir: PathBuf,
    base: String,
}

impl Home {
    fn new(name: &str, stub: &Stub) -> Self {
        let dir = std::env::temp_dir().join(format!("loker-e2e-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Home {
            dir,
            base: stub.base.clone(),
        }
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_loker"));
        c.args(args)
            .env("LOKER_HOME", &self.dir)
            .env("LOKER_FETCH_BASE", &self.base)
            .env("RUST_LOG", "error");
        c
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cmd(args).output().unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "loker {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }

    fn ingest(&self, source: &str) -> Value {
        let mut args = vec!["ingest", source, "--json"];
        if source == "himalayas" {
            args.extend(["--country", "ID", "--pages", "1"]);
        }
        self.json(&args)
    }

    /// Active jobs as sets of "source url" (ids depend on arrival order; these don't).
    fn clusters(&self) -> BTreeSet<Vec<String>> {
        let r = self.json(&["search", "--limit", "100", "--json"]);
        r["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|j| {
                let mut v: Vec<String> = j["sources"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|s| {
                        format!(
                            "{} {}",
                            s["source"].as_str().unwrap(),
                            s["url"].as_str().unwrap()
                        )
                    })
                    .collect();
                v.sort();
                v
            })
            .collect()
    }

    /// One MCP session over stdio: initialize, then call a tool.
    fn mcp_call(&self, tool: &str, arguments: Value) -> Value {
        let mut child = self
            .cmd(&["mcp"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut send = |m: Value| writeln!(stdin, "{m}").unwrap();
        let mut recv = |id: u64| loop {
            let mut line = String::new();
            assert!(
                stdout.read_line(&mut line).unwrap() > 0,
                "mcp server closed"
            );
            let m: Value = serde_json::from_str(&line).unwrap();
            if m["id"] == id {
                return m;
            }
        };
        send(
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "e2e", "version": "0"}}}),
        );
        recv(1);
        send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        send(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                    "params": {"name": tool, "arguments": arguments}}));
        let result = recv(2)["result"].clone();
        drop(stdin);
        let _ = child.wait();
        result
    }
}

const ORDER: [&str; 6] = [
    "himalayas",
    "greenhouse:acme",
    "ashby:globex",
    "lever:initech",
    "jobicy",
    "remoteok",
];

// ---------- tests ----------

#[test]
fn pipeline_end_to_end() {
    let stub = stub();
    let home = Home::new("forward", &stub);

    // Every adapter parses its payload; three pairs merge across sources.
    let first: Vec<Value> = ORDER.iter().map(|s| home.ingest(s)).collect();
    let new: u64 = first.iter().map(|s| s["new"].as_u64().unwrap()).sum();
    let skipped: u64 = first.iter().map(|s| s["skipped"].as_u64().unwrap()).sum();
    assert_eq!((new, skipped), (10, 0), "{first:#?}");
    let clusters = home.clusters();
    assert_eq!(
        clusters.len(),
        7,
        "10 listings, 3 cross-source merges: {clusters:#?}"
    );
    assert!(
        clusters.iter().any(|c| c.len() == 2
            && c[0].starts_with("jobicy")
            && c[1].starts_with("lever:initech"))
    );
    assert!(clusters.iter().any(|c| c.len() == 2
        && c[0].starts_with("greenhouse:acme")
        && c[1].starts_with("himalayas")));
    assert!(
        clusters.iter().any(|c| c.len() == 2
            && c[0].starts_with("ashby:globex")
            && c[1].starts_with("remoteok"))
    );

    // Re-running changes nothing.
    for (s, stats) in ORDER.iter().zip(ORDER.iter().map(|s| home.ingest(s))) {
        assert_eq!(
            (stats["new"].as_u64(), stats["updated"].as_u64()),
            (Some(0), Some(0)),
            "{s}: {stats}"
        );
    }

    // The same data in the opposite order gives the same jobs.
    let reverse = Home::new("reverse", &stub);
    for s in ORDER.iter().rev() {
        reverse.ingest(s);
    }
    assert_eq!(
        reverse.clusters(),
        clusters,
        "dedup must not depend on ingest order"
    );

    // The merged job takes the employer board's link and the confident eligibility.
    let r = home.json(&[
        "search",
        "backend",
        "--country",
        "ID",
        "--tz",
        "7",
        "--json",
    ]);
    let job = &r["jobs"][0];
    assert_eq!(
        job["url"],
        "https://job-boards.greenhouse.io/acme/jobs/101?gh_src=x"
    );
    assert_eq!(job["eligibility"]["scope"], "worldwide");
    assert_eq!(job["eligible"]["kind"], "yes");

    // Remote or on-site unstated, but tied to its countries: maybe inside
    // them, and dropped (no) for a viewer elsewhere.
    let r = home.json(&["search", "analyst", "--country", "DE", "--json"]);
    assert_eq!(r["jobs"][0]["eligible"]["kind"], "maybe", "{r:#}");
    let r = home.json(&["search", "analyst", "--country", "ID", "--json"]);
    assert_eq!(r["count"], 0, "{r:#}");

    // No job has both words: jobs with either come back, marked partial.
    let mcp = home.mcp_call("search_jobs", json!({"q": "writer office"}));
    let found: Vec<(&str, bool)> = mcp["structuredContent"]["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| (j["title"].as_str().unwrap(), j["partial"] == true))
        .collect();
    assert!(
        found.contains(&("Writer", true)) && found.contains(&("Office Manager", true)),
        "{found:?}"
    );
    let cli = home.json(&["search", "writer", "office", "--json"]);
    assert_eq!(mcp["structuredContent"], cli, "CLI and MCP differ");

    // Lever: an on-site job takes the posting's country, and a yearly salary
    // becomes USD per month.
    let r = home.json(&["search", "office", "--json"]);
    let job = &r["jobs"][0];
    assert_eq!(job["eligibility"]["scope"], "onsite");
    assert_eq!(job["eligibility"]["countries"], json!(["US"]));
    assert_eq!(job["pay"]["usdMonthMin"], 5000);
    assert_eq!(job["employmentType"], "full_time");

    // Every adapter's jobs go through the shared cleanup.
    let r = home.json(&["search", "designer", "--json"]);
    let job = &r["jobs"][0];
    assert_eq!(job["title"], "Designer & Illustrator");
    assert_eq!(job["company"], "Other");
    assert_eq!(
        job["url"],
        "https://himalayas.app/companies/other/jobs/designer"
    );

    // CLI and MCP return the same thing.
    let args = [
        "search",
        "engineer",
        "--country",
        "Indonesia",
        "--tz",
        "7",
        "--json",
    ];
    let cli = home.json(&args);
    let mcp = home.mcp_call(
        "search_jobs",
        json!({"q": "engineer", "country": "Indonesia", "tz": 7}),
    );
    assert_eq!(mcp["isError"], false);
    assert_eq!(
        mcp["structuredContent"], cli,
        "CLI --json and MCP structured content differ"
    );
    let bad = home.mcp_call("search_jobs", json!({"country": "Narnia"}));
    assert_eq!(bad["isError"], true);

    // An agent's flow: browse compact, eligible only; then full details for the picks.
    let browse = json!({"country": "ID", "tz": 7, "minVerdict": "yes", "view": "compact"});
    let compact = home.mcp_call("search_jobs", browse.clone())["structuredContent"].clone();
    let cli = home.json(&[
        "search",
        "--country",
        "ID",
        "--tz",
        "7",
        "--min-verdict",
        "yes",
        "--compact",
        "--json",
    ]);
    assert_eq!(compact, cli, "compact search: CLI and MCP differ");
    let hits = compact["jobs"].as_array().unwrap();
    assert!(!hits.is_empty());
    assert!(
        hits.iter().all(|h| h["eligible"]["kind"] == "yes"),
        "{compact:#}"
    );
    assert!(
        hits.iter()
            .all(|h| h.get("techTags").is_none() && h.get("sources").is_none())
    );
    let full = home.json(&[
        "search",
        "--country",
        "ID",
        "--tz",
        "7",
        "--min-verdict",
        "yes",
        "--json",
    ]);
    let (c, f) = (compact.to_string().len(), full.to_string().len());
    assert!(
        c * 2 < f,
        "compact ({c} bytes) should be well under half of full ({f})"
    );

    let ids: Vec<Value> = hits.iter().map(|h| h["id"].clone()).collect();
    let mut ask = ids.clone();
    ask.push(json!("j_nope"));
    let got = home.mcp_call("get_jobs", json!({"ids": ask, "country": "ID", "tz": 7}))["structuredContent"].clone();
    let got_ids: Vec<Value> = got["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["id"].clone())
        .collect();
    assert_eq!(got_ids, ids, "get_jobs returns the picks in order");
    assert_eq!(got["missing"], json!(["j_nope"]));
    assert!(got["jobs"][0]["eligibility"]["confidence"].is_number());
    let mut args = vec!["get".to_string()];
    args.extend(ids.iter().map(|i| i.as_str().unwrap().to_string()));
    args.extend(["j_nope", "--country", "ID", "--tz", "7", "--json"].map(String::from));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    assert_eq!(home.json(&args), got, "get_jobs: CLI and MCP differ");

    // A verdict filter needs a viewer.
    let bad = home.mcp_call("search_jobs", json!({"minVerdict": "yes"}));
    assert_eq!(bad["isError"], true);

    // A complete board that stops listing a job closes it; listing it again reopens it.
    stub.shrink.store(true, Ordering::SeqCst);
    let st = home.ingest("ashby:globex");
    assert_eq!(
        (st["closed"].as_u64(), st["held"].as_u64()),
        (Some(1), Some(0)),
        "{st}"
    );
    assert_eq!(home.clusters().len(), clusters.len() - 1);
    stub.shrink.store(false, Ordering::SeqCst);
    let st = home.ingest("ashby:globex");
    assert_eq!(st["reopened"].as_u64(), Some(1), "{st}");
    assert_eq!(home.clusters(), clusters);
}

#[test]
fn verify_passes_healthy_sources_and_fails_broken_ones() {
    let stub = stub();
    let home = Home::new("verify", &stub);
    for s in ORDER {
        home.ingest(s);
    }
    let ok = home.run(&["verify", "--json"]);
    let report: Value = serde_json::from_slice(&ok.stdout).unwrap();
    assert!(ok.status.success(), "{report:#}");
    assert_eq!(report["ok"], true);
    assert_eq!(report["sources"].as_array().unwrap().len(), ORDER.len());

    let broken = home.run(&["verify", "greenhouse:broken", "--json"]);
    let report: Value = serde_json::from_slice(&broken.stdout).unwrap();
    assert!(
        !broken.status.success(),
        "a failing source must fail the command"
    );
    assert_eq!(report["sources"][0]["status"], "fail");
    assert_eq!(report["sources"][0]["checks"][0]["name"], "fetch");
}

#[test]
fn contract_schemas_match_committed_v1() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/v1");
    let mut drift = Vec::new();
    for (name, schema) in loker::contract_schemas() {
        let committed: Option<Value> = std::fs::read_to_string(dir.join(name))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok());
        if committed.as_ref() != Some(&schema) {
            drift.push(name);
        }
    }
    assert!(
        drift.is_empty(),
        "the v1 contract changed: {drift:?}. If the change is additive, run \
         `cargo run -- schema --out schema/v1` and commit; if not, it needs a new contract version."
    );
}

#[test]
fn config_drives_ingest_all_and_verify() {
    let stub = stub();
    let home = Home::new("config", &stub);
    std::fs::create_dir_all(&home.dir).unwrap();
    std::fs::write(
        home.dir.join("config.toml"),
        r#"
version = 1

[[sources]]
id = "himalayas"
country = "ID"
pages = 1

[[sources]]
id = "greenhouse:acme"
remote = true

[[sources]]
id = "ashby:globex"
company = "Globex"

[[sources]]
id = "greenhouse:broken"

[[sources]]
id = "jobicy"

[[sources]]
id = "remoteok"
enabled = false
"#,
    )
    .unwrap();

    // One broken source fails the run but not the others; disabled ones are skipped.
    let out = home.run(&["ingest", "--all", "--json"]);
    assert!(
        !out.status.success(),
        "a failed source must fail `ingest --all`"
    );
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    let sources: Vec<&str> = report["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["source"].as_str().unwrap())
        .collect();
    assert_eq!(
        sources,
        [
            "himalayas",
            "greenhouse:acme",
            "ashby:globex",
            "greenhouse:broken",
            "jobicy"
        ]
    );
    let failed: Vec<&Value> = report["sources"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| !o["error"].is_null())
        .collect();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0]["source"], "greenhouse:broken");

    // A remote board: its postings' countries are where you can work from.
    let r = home.json(&["search", "analyst", "--country", "DE", "--json"]);
    assert_eq!(r["jobs"][0]["eligibility"]["scope"], "countries", "{r:#}");
    assert_eq!(r["jobs"][0]["eligible"]["kind"], "yes", "{r:#}");

    // The board's display name from config replaces Ashby's slug.
    let r = home.json(&["search", "support", "--json"]);
    assert_eq!(r["jobs"][0]["company"], "Globex");

    // verify with no names checks the enabled config sources with their own
    // parameters: Himalayas on the country search endpoint ingest uses.
    stub.log.lock().unwrap().clear();
    let out = home.run(&["verify", "--json"]);
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(!out.status.success());
    let checked: Vec<&str> = report["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["source"].as_str().unwrap())
        .collect();
    assert_eq!(
        checked,
        [
            "himalayas",
            "greenhouse:acme",
            "ashby:globex",
            "greenhouse:broken",
            "jobicy"
        ]
    );
    let served = stub.log.lock().unwrap().clone();
    assert!(
        served
            .iter()
            .any(|t| t.starts_with("/jobs/api/search?") && t.contains("country=ID")),
        "{served:?}"
    );
    assert!(
        !served.iter().any(|t| t.starts_with("/jobs/api?")),
        "{served:?}"
    );
}
