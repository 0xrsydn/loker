use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

struct Stub {
    base: String,
    shrink: Arc<AtomicBool>,
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
            let line = String::from_utf8_lossy(&head).to_string();
            // Read a POST body to its end, so the reply doesn't reset the connection.
            let length = line
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            let mut body = vec![0u8; length];
            let _ = stream.read_exact(&mut body);
            let target = line.split_whitespace().nth(1).unwrap_or("/").to_string();
            let times = {
                let mut served = served.lock().unwrap();
                served.push(target.clone());
                served.iter().filter(|t| **t == target).count()
            };
            if target.starts_with("/v1/boards/throttled/jobs") && times == 1 {
                let _ = write!(
                    stream,
                    "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                continue;
            }
            if let Some(bytes) = cc_block(&target) {
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                );
                let _ = stream.write_all(&bytes);
                continue;
            }
            let (status, body) = route(&target, flag.load(Ordering::SeqCst));
            let body = match body {
                Value::String(text) => text,
                other => other.to_string(),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    Stub { base, shrink, log }
}

const CC_INDEXES: &str = "/cc-index/collections/CC-MAIN-2026-43/indexes";
const CC_CLUSTER: &str = "/cc-index/collections/CC-MAIN-2026-43/indexes/cluster.idx";

fn cdx_block(records: &[(&str, &str)]) -> Vec<u8> {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    for (key, url) in records {
        writeln!(
            gz,
            "{key} 20261001 {}",
            json!({ "url": url, "status": "200" })
        )
        .unwrap();
    }
    gz.finish().unwrap()
}

fn cc_block(target: &str) -> Option<Vec<u8>> {
    match target.strip_prefix(CC_INDEXES)? {
        "/cdx-00000.gz" => Some(cdx_block(&[
            ("co,example)/", "https://example.co/"),
            (
                "co,lever,jobs)/initech/1",
                "https://jobs.lever.co/initech/1",
            ),
        ])),
        "/cdx-00001.gz" => Some(cdx_block(&[
            (
                "com,ashbyhq,jobs)/globex/a-1",
                "https://jobs.ashbyhq.com/GLOBEX/a-1",
            ),
            (
                "io,greenhouse,boards)/acme",
                "https://boards.greenhouse.io/acme",
            ),
            (
                "io,greenhouse,boards)/robots.txt",
                "https://boards.greenhouse.io/robots.txt",
            ),
        ])),
        _ => None,
    }
}

fn cc_cluster() -> String {
    let len = |file: &str| cc_block(&format!("{CC_INDEXES}/{file}")).unwrap().len();
    format!(
        "co,example)/ 20261001\tcdx-00000.gz\t0\t{}\t1\n\
         com,ashbyhq,jobs)/globex/a-1 20261001\tcdx-00001.gz\t0\t{}\t2\n\
         org,example)/ 20261001\tcdx-00002.gz\t0\t10\t3\n",
        len("cdx-00000.gz"),
        len("cdx-00001.gz")
    )
}

fn route(target: &str, shrink: bool) -> (&'static str, Value) {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    match path {
        "/jobs/api/search" if query.contains("page=1") => {
            ("200 OK", json!({ "jobs": himalayas() }))
        }
        "/jobs/api/search" => ("200 OK", json!({ "jobs": [] })),
        "/jobs/api" => ("200 OK", json!({ "jobs": himalayas(), "nextCursor": null })),
        "/v1/boards/acme/jobs" | "/v1/boards/throttled/jobs" => {
            ("200 OK", json!({ "jobs": greenhouse() }))
        }
        "/v1/boards/broken/jobs" => ("500 Internal Server Error", json!({ "error": "boom" })),
        "/posting-api/job-board/globex" => {
            let mut jobs = ashby();
            if shrink {
                jobs.pop();
            }
            ("200 OK", json!({ "apiVersion": "1", "jobs": jobs }))
        }
        "/posting-api/job-board/GLOBEX" => {
            ("200 OK", json!({ "apiVersion": "1", "jobs": ashby() }))
        }
        "/posting-api/job-board/Hippocratic%20AI" => {
            ("200 OK", json!({ "apiVersion": "1", "jobs": ashby() }))
        }
        "/collinfo.json" => ("200 OK", json!([{ "id": "CC-MAIN-2026-43" }])),
        CC_CLUSTER => ("200 OK", Value::String(cc_cluster())),
        "/api/v2/remote-jobs" => ("200 OK", json!({ "jobs": jobicy() })),
        "/v0/postings/initech" if query.contains("skip=0") => ("200 OK", Value::Array(lever())),
        "/v0/postings/initech" => ("200 OK", json!([])),
        "/api/alpha/decisions" => (
            "200 OK",
            json!({
                "model": "test/jev",
                "answers": { "role": {
                    "type": "choice", "choice": "data_engineering", "confidence": 0.9,
                    "probabilities": { "data_engineering": 0.9, "backend": 0.1 }
                }},
                "usage": { "cost": 0.0001 }
            }),
        ),
        // Only the all-jobs feed: the category feeds 404 and are skipped.
        "/remote-jobs.rss" => ("200 OK", Value::String(weworkremotely())),
        "/jobs/sitemap.xml" => ("200 OK", Value::String(yc_sitemap())),
        "/companies/acme/jobs" => ("200 OK", Value::String(yc_page("Acme", yc_jobs().0))),
        "/companies/globex/jobs" => ("200 OK", Value::String(yc_page("Globex", yc_jobs().1))),
        "/api" => {
            let mut items = vec![json!({ "legal": "API terms: link back to Remote OK." })];
            items.extend(remoteok());
            ("200 OK", Value::Array(items))
        }
        _ => ("404 Not Found", json!({})),
    }
}

fn yc_sitemap() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?><urlset>
  <url><loc>https://www.ycombinator.com/companies/acme/jobs/abc-sre</loc></url>
  <url><loc>https://www.ycombinator.com/companies/acme/jobs/def-platform</loc></url>
  <url><loc>https://www.ycombinator.com/companies/globex/jobs/ghi-designer</loc></url>
</urlset>"#
        .to_string()
}

fn yc_page(name: &str, jobs: Value) -> String {
    let props = json!({
        "component": "WaasShowJobsPage",
        "props": { "jobPostings": jobs, "company": { "name": name } },
    });
    let escaped = props.to_string().replace('"', "&quot;");
    format!(
        "<html><body><div id=\"WaasShowJobsPage-react-component-1\" data-page=\"{escaped}\"></div></body></html>"
    )
}

fn yc_jobs() -> (Value, Value) {
    (
        json!([
            {
                "id": 900001,
                "title": "Site Reliability Engineer",
                "url": "/companies/acme/jobs/abc-sre",
                "companyName": "Acme",
                "location": "San Francisco, CA, US / Remote (US)",
                "type": "Full-time",
                "prettyRole": "Engineering",
                "roleSpecificType": "Backend",
                "salaryRange": "$180K - $210K",
                "equityRange": "0.10% - 0.30%",
                "visa": "Will sponsor",
                "skills": ["Kubernetes", "Go"],
                "createdAt": "2 days"
            },
            {
                "id": 900002,
                "title": "Platform Engineer",
                "url": "/companies/acme/jobs/def-platform",
                "companyName": "Acme",
                "location": "Remote (US; CA)",
                "type": "Contract",
                "prettyRole": "Engineering",
                "salaryRange": "$1",
                "visa": "US citizenship/visa not required",
                "createdAt": "about 2 months"
            }
        ]),
        json!([{
            "id": 900003,
            "title": "Product Designer",
            "url": "/companies/globex/jobs/ghi-designer",
            "companyName": "Globex",
            "location": "London, England, GB",
            "type": "Full-time",
            "prettyRole": "Design",
            "visa": "US citizen/visa only",
            "createdAt": "1 day"
        }]),
    )
}

fn weworkremotely() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss"><channel>
<title>We Work Remotely</title>
<item>
  <media:content url="https://example.invalid/logo.png" type="image/png"/>
  <title>Initrode: Site Reliability Engineer</title>
  <region>Asia Only</region>
  <category>DevOps and Sysadmin</category>
  <type>Full-Time</type>
  <skills>Kubernetes, Terraform, and Go</skills>
  <description>&lt;p&gt;Keep &lt;b&gt;production&lt;/b&gt; up.&lt;/p&gt;</description>
  <pubDate>Thu, 19 Nov 2026 16:48:31 +0000</pubDate>
  <expires_at>Fri, 01 Jan 2100 00:00:00 +0000</expires_at>
  <guid>https://weworkremotely.com/remote-jobs/initrode-site-reliability-engineer</guid>
  <link>https://weworkremotely.com/remote-jobs/initrode-site-reliability-engineer</link>
</item>
</channel></rss>"#
        .into()
}

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
            "descriptionHtml": "<p>React.</p><p>Visa sponsorship is not available.</p>",
            "compensation": {
                "compensationTierSummary": "$100K – $130K • Offers Equity",
                "compensationTiers": [{ "title": null, "components": [
                    { "compensationType": "Salary", "currencyCode": "USD", "interval": "1 YEAR",
                      "minValue": 100000, "maxValue": 130000 },
                    { "compensationType": "EquityPercentage", "currencyCode": null, "interval": "NONE",
                      "minValue": null, "maxValue": null }
                ]}]
            }
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
        json!({
            "id": "lv-1", "text": "Writer", "workplaceType": "remote", "country": "US",
            "categories": { "commitment": "Full-time", "location": "Anywhere", "allLocations": ["Anywhere"] },
            "hostedUrl": "https://jobs.lever.co/initech/lv-1",
            "applyUrl": "https://jobs.lever.co/initech/lv-1/apply",
            "createdAt": 1_790_000_000_000_i64, "description": "<p>Words.</p>",
            "lists": [{ "text": "You have", "content": "<li>A pen</li>" }], "additional": ""
        }),
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
            .env("OPENROUTER_API_KEY", "test-key")
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

const ORDER: [&str; 7] = [
    "himalayas",
    "greenhouse:acme",
    "ashby:globex",
    "lever:initech",
    "jobicy",
    "remoteok",
    "weworkremotely",
];

#[test]
fn pipeline_end_to_end() {
    let stub = stub();
    let home = Home::new("forward", &stub);

    let r = home.json(&["search", "engineer", "--json"]);
    assert!(
        r["notice"].as_str().unwrap().contains("loker ingest --all"),
        "before any ingest, a search says why it is empty: {r:#}"
    );

    let first: Vec<Value> = ORDER.iter().map(|s| home.ingest(s)).collect();
    let new: u64 = first.iter().map(|s| s["new"].as_u64().unwrap()).sum();
    let skipped: u64 = first.iter().map(|s| s["skipped"].as_u64().unwrap()).sum();
    assert_eq!((new, skipped), (11, 0), "{first:#?}");
    let clusters = home.clusters();
    assert_eq!(
        clusters.len(),
        8,
        "11 listings, 3 cross-source merges: {clusters:#?}"
    );
    assert!(
        clusters.iter().any(|c| c.len() == 2
            && c[0].starts_with("jobicy")
            && c[1].starts_with("lever:initech")),
        "Jobicy's and Lever's Writer are one job"
    );
    assert!(clusters.iter().any(|c| c.len() == 2
        && c[0].starts_with("greenhouse:acme")
        && c[1].starts_with("himalayas")));
    assert!(
        clusters.iter().any(|c| c.len() == 2
            && c[0].starts_with("ashby:globex")
            && c[1].starts_with("remoteok"))
    );

    for (s, stats) in ORDER.iter().zip(ORDER.iter().map(|s| home.ingest(s))) {
        assert_eq!(
            (stats["new"].as_u64(), stats["updated"].as_u64()),
            (Some(0), Some(0)),
            "{s}: re-running changes nothing: {stats}"
        );
    }

    let reverse = Home::new("reverse", &stub);
    for s in ORDER.iter().rev() {
        reverse.ingest(s);
    }
    assert_eq!(
        reverse.clusters(),
        clusters,
        "dedup must not depend on ingest order"
    );

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
        job["url"], "https://job-boards.greenhouse.io/acme/jobs/101?gh_src=x",
        "the merged job takes the employer board's link"
    );
    assert_eq!(
        job["eligibility"]["scope"], "worldwide",
        "and the confident eligibility"
    );
    assert_eq!(job["eligible"]["kind"], "yes");

    let r = home.json(&["search", "analyst", "--country", "DE", "--json"]);
    assert_eq!(
        r["jobs"][0]["eligible"]["kind"], "maybe",
        "Greenhouse lists countries without saying remote: maybe inside them: {r:#}"
    );
    let r = home.json(&["search", "analyst", "--country", "ID", "--json"]);
    assert_eq!(r["count"], 0, "and dropped (no) elsewhere: {r:#}");

    let mcp = home.mcp_call("search_jobs", json!({"q": "writer office"}));
    let found: Vec<(&str, bool)> = mcp["structuredContent"]["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| (j["title"].as_str().unwrap(), j["partial"] == true))
        .collect();
    assert!(
        found.contains(&("Writer", true)) && found.contains(&("Office Manager", true)),
        "no job has both words: jobs with either come back, marked partial: {found:?}"
    );
    let cli = home.json(&["search", "writer", "office", "--json"]);
    assert_eq!(mcp["structuredContent"], cli, "CLI and MCP differ");

    let cli = home.json(&["roles", "--json"]);
    assert_eq!(
        home.mcp_call("list_roles", json!({}))["structuredContent"],
        cli,
        "roles: CLI and MCP differ"
    );
    let r = home.json(&["search", "--role", "infrastructure", "--compact", "--json"]);
    assert!(
        r["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|j| j["title"] == "Site Reliability Engineer" && j["roles"][0] == "infrastructure"),
        "the WWR job's title places it: {r:#}"
    );

    let r = home.json(&["search", "reliability", "--country", "ID", "--json"]);
    let job = &r["jobs"][0];
    assert_eq!(
        (&job["company"], &job["title"], &job["eligible"]["kind"]),
        (
            &json!("Initrode"),
            &json!("Site Reliability Engineer"),
            &json!("yes")
        ),
        "WWR: \"Company: Role\" splits, and its region is who may apply: {r:#}"
    );

    let r = home.json(&["search", "office", "--json"]);
    let job = &r["jobs"][0];
    assert_eq!(job["eligibility"]["scope"], "onsite");
    assert_eq!(
        job["eligibility"]["countries"],
        json!(["US"]),
        "Lever: an on-site job takes the posting's country"
    );
    assert_eq!(
        job["pay"]["usdMonthMin"], 5000,
        "Lever: a yearly salary becomes USD per month"
    );

    let r = home.json(&["search", "frontend", "--json"]);
    let pay = &r["jobs"][0]["pay"];
    assert_eq!(
        (&pay["base"], &pay["extras"], &pay["raw"]),
        (
            &json!([{ "min": 100000.0, "max": 130000.0, "currency": "USD", "period": "year" }]),
            &json!([{ "kind": "equity" }]),
            &json!("$100K – $130K • Offers Equity")
        ),
        "Ashby: pay from its components; the employer's wins over Remote OK's: {r:#}"
    );
    assert_eq!(
        r["jobs"][0]["visa"]["sponsorship"], "not_offered",
        "visa sponsorship is read from the description: {r:#}"
    );
    assert_eq!(
        r["jobs"][0]["employmentType"], "full_time",
        "Ashby: employmentType \"FullTime\" reads as full_time: {r:#}"
    );

    let r = home.json(&["search", "designer", "--json"]);
    let job = &r["jobs"][0];
    assert_eq!(
        job["title"], "Designer & Illustrator",
        "the stub's messy title goes through normalize::tidy"
    );
    assert_eq!(job["company"], "Other");
    assert_eq!(
        job["url"],
        "https://himalayas.app/companies/other/jobs/designer"
    );

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

    let bad = home.mcp_call("search_jobs", json!({"minVerdict": "yes"}));
    assert_eq!(bad["isError"], true, "a verdict filter needs a viewer");

    stub.shrink.store(true, Ordering::SeqCst);
    let st = home.ingest("ashby:globex");
    assert_eq!(
        (st["closed"].as_u64(), st["held"].as_u64()),
        (Some(1), Some(0)),
        "a complete board that stops listing a job closes it: {st}"
    );
    assert_eq!(home.clusters().len(), clusters.len() - 1);
    stub.shrink.store(false, Ordering::SeqCst);
    let st = home.ingest("ashby:globex");
    assert_eq!(
        st["reopened"].as_u64(),
        Some(1),
        "listing it again reopens it: {st}"
    );
    assert_eq!(home.clusters(), clusters);
}

#[test]
fn ycombinator_reads_the_sitemap_then_each_company_page() {
    let stub = stub();
    let home = Home::new("yc", &stub);
    let st = home.ingest("ycombinator");
    assert_eq!(st["new"].as_u64(), Some(3), "{st}");
    assert_eq!(st["skipped"].as_u64(), Some(0), "{st}");

    let served = stub.log.lock().unwrap().clone();
    assert_eq!(
        served,
        [
            "/jobs/sitemap.xml",
            "/companies/acme/jobs",
            "/companies/globex/jobs"
        ],
        "the sitemap first, then one page per company it named"
    );

    let again = home.ingest("ycombinator");
    assert_eq!(
        (again["updated"].as_u64(), again["unchanged"].as_u64()),
        (Some(0), Some(3)),
        "the same board again changes nothing, though its dates are ages: {again}"
    );

    let found = home.json(&["search", "--limit", "10", "--json"]);
    let jobs = found["jobs"].as_array().unwrap();
    let sre = jobs
        .iter()
        .find(|j| j["title"] == "Site Reliability Engineer")
        .expect("the SRE posting");
    assert_eq!(sre["company"], "Acme");
    assert_eq!(
        sre["url"],
        "https://www.ycombinator.com/companies/acme/jobs/abc-sre"
    );
    assert_eq!(sre["visa"]["sponsorship"], "offered");
    assert_eq!(sre["pay"]["base"][0]["min"], 180000.0);
    assert_eq!(sre["pay"]["base"][0]["max"], 210000.0);
    assert_eq!(sre["pay"]["base"][0]["currency"], "USD");

    let platform = jobs
        .iter()
        .find(|j| j["title"] == "Platform Engineer")
        .expect("the platform posting");
    assert!(
        platform["pay"].is_null(),
        "the board's \"$1\" placeholder is not pay: {platform:#}"
    );
    assert!(
        platform["visa"].is_null(),
        "\"not required\" says nothing about sponsorship: {platform:#}"
    );

    let ok = home.run(&["verify", "ycombinator", "--json"]);
    let report: Value = serde_json::from_slice(&ok.stdout).unwrap();
    assert!(ok.status.success(), "{report:#}");
    assert_eq!(report["sources"][0]["status"], "pass");
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
fn an_ashby_slug_keeps_its_spaces_and_case() {
    let stub = stub();
    let home = Home::new("ashby-space", &stub);
    let r = home.ingest("ashby:Hippocratic AI");
    assert!(r["new"].as_u64().unwrap() > 0, "{r:#}");
    let bad = home.run(&["ingest", "ashby:a/b", "--json"]);
    assert!(!bad.status.success(), "a slash is never part of a slug");
}

#[test]
fn a_host_that_asks_to_slow_down_is_paused_then_asked_again() {
    let stub = stub();
    let home = Home::new("throttled", &stub);
    let started = std::time::Instant::now();
    let r = home.ingest("greenhouse:throttled");
    assert!(r["new"].as_u64().unwrap() > 0, "{r:#}");
    assert!(started.elapsed() >= std::time::Duration::from_secs(1));
    let asked = stub
        .log
        .lock()
        .unwrap()
        .iter()
        .filter(|t| t.starts_with("/v1/boards/throttled/jobs"))
        .count();
    assert_eq!(asked, 2);
}

#[test]
fn discover_checks_each_board_once_and_asks_again_when_unanswered() {
    let stub = stub();
    let home = Home::new("discover", &stub);
    std::fs::create_dir_all(&home.dir).unwrap();
    let list = home.dir.join("boards.txt");
    std::fs::write(
        &list,
        "https://jobs.ashbyhq.com/GLOBEX/a-1\n\
         ashby:globex\n\
         https://boards.greenhouse.io/embed/job_app?for=acme\n\
         greenhouse:broken\n\
         lever:initech\n\
         ashby:nobody\n\
         https://example.com/careers\n\n",
    )
    .unwrap();
    let args = ["discover", "--from", list.to_str().unwrap(), "--json"];
    let r = home.json(&args);
    assert_eq!(r["finder"], "file:boards.txt");
    assert_eq!(
        (&r["found"], &r["new"], &r["unreadable"]),
        (&json!(5), &json!(5), &json!(1)),
        "one board under two spellings counts once: {r:#}"
    );
    assert_eq!(
        r["checked"],
        json!({ "live": 2, "empty": 1, "missing": 1, "unchecked": 0, "failed": 1 })
    );

    let again = home.json(&args);
    assert_eq!(again["new"], 0);
    assert_eq!(
        again["checked"],
        json!({ "live": 0, "empty": 0, "missing": 0, "unchecked": 0, "failed": 1 }),
        "only the board that got no answer is asked again"
    );
    assert_eq!(
        again["boards"],
        json!({ "live": 2, "empty": 1, "missing": 1, "unchecked": 1, "failed": 1 })
    );
}

#[test]
fn discover_reads_the_newest_common_crawl_once() {
    let stub = stub();
    let home = Home::new("commoncrawl", &stub);
    let r = home.json(&["discover", "--json"]);
    assert_eq!(r["finder"], "commoncrawl:CC-MAIN-2026-43");
    assert_eq!(
        (&r["found"], &r["unreadable"]),
        (&json!(3), &json!(1)),
        "{r:#}"
    );
    assert_eq!(
        r["checked"],
        json!({ "live": 2, "empty": 1, "missing": 0, "unchecked": 0, "failed": 0 })
    );
    assert!(
        !stub
            .log
            .lock()
            .unwrap()
            .iter()
            .any(|t| t.contains("cdx-00002")),
        "a block that can't hold a board host is never read"
    );
    let again = home.json(&["discover", "--json"]);
    assert_eq!(
        (&again["skipped"], &again["found"]),
        (&json!(true), &json!(0))
    );
}

#[test]
fn ingest_all_takes_live_discovered_boards_not_already_listed() {
    let stub = stub();
    let home = Home::new("discovered", &stub);
    std::fs::create_dir_all(&home.dir).unwrap();
    std::fs::write(
        home.dir.join("config.toml"),
        "version = 1\n\n[[sources]]\nid = \"greenhouse:acme\"\n\n[discovery]\ningest = true\n",
    )
    .unwrap();
    let list = home.dir.join("boards.txt");
    std::fs::write(
        &list,
        "https://jobs.ashbyhq.com/GLOBEX/a-1\n\
         https://boards.greenhouse.io/ACME\n\
         lever:initech\n",
    )
    .unwrap();
    home.json(&["discover", "--from", list.to_str().unwrap(), "--json"]);

    let report = home.json(&["ingest", "--all", "--json"]);
    let sources: Vec<&str> = report["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["source"].as_str().unwrap())
        .collect();
    assert_eq!(
        sources,
        ["greenhouse:acme", "ashby:globex"],
        "a listed board isn't fetched twice, an empty one not at all, and Ashby's spelling wins"
    );
    assert_eq!(report["ok"], true, "{report:#}");
}

#[test]
fn every_source_is_named_in_the_schema_and_the_help() {
    let schemas = loker::contract_schemas();
    let ingest = &schemas
        .iter()
        .find(|(name, _)| *name == "ingest_params.json")
        .expect("ingest_params schema")
        .1;
    let described = ingest["properties"]["source"]["description"]
        .as_str()
        .unwrap();
    let help = Command::new(env!("CARGO_BIN_EXE_loker"))
        .args(["ingest", "--help"])
        .output()
        .unwrap();
    let help = String::from_utf8(help.stdout).unwrap();
    let missing: Vec<_> = loker_sources::SOURCES
        .iter()
        .filter(|s| !described.contains(*s) || !help.contains(*s))
        .collect();
    assert!(
        missing.is_empty(),
        "sources missing from IngestParams.source or `ingest --help`: {missing:?}"
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
        ],
        "disabled sources are skipped"
    );
    let failed: Vec<&Value> = report["sources"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| !o["error"].is_null())
        .collect();
    assert_eq!(failed.len(), 1, "one broken source doesn't fail the others");
    assert_eq!(failed[0]["source"], "greenhouse:broken");

    let r = home.json(&["search", "analyst", "--country", "DE", "--json"]);
    assert_eq!(
        r["jobs"][0]["eligibility"]["scope"], "countries",
        "a remote board's countries are where you can work from: {r:#}"
    );
    assert_eq!(r["jobs"][0]["eligible"]["kind"], "yes", "{r:#}");

    let r = home.json(&["search", "support", "--json"]);
    assert_eq!(
        r["jobs"][0]["company"], "Globex",
        "the display name from config replaces Ashby's slug"
    );

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
        ],
        "verify with no names checks the enabled config sources"
    );
    let served = stub.log.lock().unwrap().clone();
    assert!(
        served
            .iter()
            .any(|t| t.starts_with("/jobs/api/search?") && t.contains("country=ID")),
        "verify uses each source's config parameters, as ingest does: {served:?}"
    );
    assert!(
        !served.iter().any(|t| t.starts_with("/jobs/api?")),
        "{served:?}"
    );

    let out = home.run(&["doctor", "--json"]);
    assert!(!out.status.success());
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    let status = |name: &str| {
        report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .map(|c| (c["status"].as_str().unwrap().to_string(), c["fix"].clone()))
            .unwrap()
    };
    assert_eq!(status("source greenhouse:broken").0, "fail", "{report:#}");
    assert_eq!(
        status("source greenhouse:broken").1,
        "loker verify greenhouse:broken"
    );
    assert_eq!(status("source jobicy").0, "pass", "{report:#}");
    assert_eq!(status("database").0, "pass", "{report:#}");
}

#[test]
fn a_model_classifies_roles_once_per_job() {
    let stub = stub();
    let home = Home::new("jev", &stub);
    for s in ORDER {
        home.ingest(s);
    }
    let without = home.run(&["roles", "enrich", "--json"]);
    assert!(!without.status.success(), "no [providers.jev] is an error");

    std::fs::write(
        home.dir.join("config.toml"),
        "version = 1\n[providers.jev]\nmodel = \"test/jev\"\n",
    )
    .unwrap();
    let jobs = home.json(&["stats", "--json"])["total"].as_u64().unwrap();
    let r = home.json(&["roles", "enrich", "--json"]);
    assert_eq!(
        (
            r["asked"].as_u64(),
            r["answered"].as_u64(),
            r["failed"].as_u64()
        ),
        (Some(jobs), Some(jobs), Some(0)),
        "{r:#}"
    );

    let found = home.json(&["search", "--role", "data_engineering", "--json"]);
    assert_eq!(
        found["total"].as_u64(),
        Some(jobs),
        "the stub's answer (data_engineering, 0.9) wins over every title: {found:#}"
    );
    assert_eq!(found["jobs"][0]["role"]["method"], "model");

    let again = home.json(&["roles", "enrich", "--json"]);
    assert_eq!(
        again["asked"].as_u64(),
        Some(0),
        "answers are cached per job content: {again:#}"
    );
}
