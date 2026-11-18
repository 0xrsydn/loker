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
            "title": "Designer", "companyName": "Other",
            "applicationLink": "https://himalayas.app/companies/other/jobs/designer",
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
            "id": 102, "title": "Data Analyst", "company_name": "Acme, Inc.",
            "absolute_url": "https://job-boards.greenhouse.io/acme/jobs/102",
            "location": { "name": "Berlin" },
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

const ORDER: [&str; 5] = [
    "himalayas",
    "greenhouse:acme",
    "ashby:globex",
    "jobicy",
    "remoteok",
];

// ---------- tests ----------

#[test]
fn pipeline_end_to_end() {
    let stub = stub();
    let home = Home::new("forward", &stub);

    // Every adapter parses its payload; two pairs merge across sources.
    let first: Vec<Value> = ORDER.iter().map(|s| home.ingest(s)).collect();
    let new: u64 = first.iter().map(|s| s["new"].as_u64().unwrap()).sum();
    let skipped: u64 = first.iter().map(|s| s["skipped"].as_u64().unwrap()).sum();
    assert_eq!((new, skipped), (8, 0), "{first:#?}");
    let clusters = home.clusters();
    assert_eq!(
        clusters.len(),
        6,
        "8 listings, 2 cross-source merges: {clusters:#?}"
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

    // A complete board that stops listing a job closes it; listing it again reopens it.
    stub.shrink.store(true, Ordering::SeqCst);
    let st = home.ingest("ashby:globex");
    assert_eq!(
        (st["closed"].as_u64(), st["held"].as_u64()),
        (Some(1), Some(0)),
        "{st}"
    );
    assert_eq!(home.clusters().len(), 5);
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
