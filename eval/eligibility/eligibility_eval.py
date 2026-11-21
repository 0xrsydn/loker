"""Eligibility eval for loker: the verdict (yes / maybe / unknown / no) the
current rules give five viewers, against verdicts labelled by Opus and
DeepSeek from the whole posting. Run by hand; not part of CI. See README.md.

  python3 eligibility_eval.py parity [--db PATH]   # rebuilt eligibility vs what the snapshot stored
  python3 eligibility_eval.py sample               # draw sample.json (tuning) and holdout.json, once
  python3 eligibility_eval.py sample2              # draw holdout2.json from snapshot-h2.db, once
  python3 eligibility_eval.py sample-workable      # draw workable.json (a new source), once
  JUDGE=opus|deepseek41 python3 eligibility_eval.py label [--set holdout]   # deepseek: the old API judge
  python3 eligibility_eval.py agree [--set holdout] [--with deepseek41]
  python3 eligibility_eval.py score [--set holdout] [-v]

Verdicts come from `crates/loker-sources/examples/eligibility.rs`, which
re-reads every listing's stored raw item with the current adapters and
rules, so a rule change is measured without fetching again. Never print a
holdout job's text or labels while writing rules; add any job you read to
seen.json.
"""
import json, os, random, re, sqlite3, subprocess, sys, tempfile, time, urllib.request
from collections import Counter, defaultdict
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
EVAL = os.path.dirname(HERE)
REPO = os.path.dirname(EVAL)
# Holdout: aggregators fetched 2026-11-20, after the tuning snapshot, and
# the 36 employer boards first fetched for the visa holdouts.
DBS = {"sample": [os.path.join(EVAL, "pay", "snapshot.db")],
       "holdout": [os.path.join(HERE, "snapshot-agg.db"), os.path.join(EVAL, "visa", "snapshot-boards.db"),
                   os.path.join(EVAL, "visa", "snapshot-boards2.db")],
       # holdout2: a fetch on 2026-11-20 14:30, after the description reader
       # was written (the aggregators again, and 21 employer boards new to
       # loker), and snapshot-agg's jobs that no set drew and nobody read.
       # Himalayas pages 1-50 fetched with it: the first 10 were in older snapshots.
       # A new source's first test: Workable, fetched 2026-11-20 15:10 (10
       # pages, and 5 each for PH, ID, IN, DE), never used to write rules.
       "workable": [os.path.join(HERE, "snapshot-workable.db")],
       # After the Workable rules were written on `workable`: a deeper fetch
       # (30 pages, 10 each for PH, ID, IN, DE), minus every job `workable` has.
       "workable2": [os.path.join(HERE, "snapshot-workable2.db")],
       # After the second round of Workable rules (Location lines, the
       # worldwide bug): 50 pages, 15 each for PH, ID, IN, DE, minus every job
       # `workable` and `workable2` have.
       "workable3": [os.path.join(HERE, "snapshot-workable3.db")],
       # After the workplace/places/elsewhere split: BR, GB, US, 10 pages each,
       # minus every job of the earlier Workable sets.
       "workable4": [os.path.join(HERE, "snapshot-workable4.db")],
       # Y Combinator: the parser was written from snapshot-yc.db's 121 jobs;
       # yc.json is drawn from a later fetch (80 company pages) minus those.
       "yc": [os.path.join(HERE, "snapshot-yc2.db")],
       "holdout2": [os.path.join(HERE, "snapshot-h2.db"), os.path.join(HERE, "snapshot-agg.db"),
                    os.path.join(HERE, "snapshot-h2-himalayas.db")]}
BIN = os.path.join(REPO, "target", "release", "examples", "eligibility")
PROMPT_VERSION = "v1"
# deepseek: DeepSeek's own API (v4 Pro), used for the sets labelled before
# 2026-11-20 16:30, when its credit ran out. deepseek41: DeepSeek v4.1 Flash
# through OpenCode Go in Pi, high reasoning; its labels are kept apart.
JUDGES = {"opus": "claude-opus-5-5", "deepseek": "deepseek-v4-pro", "deepseek41": "opencode-go/deepseek-v4.1-flash:high"}
MAX_CHARS = 12000
# November: standard time everywhere these viewers live.
VIEWERS = [("US", -5.0), ("DE", 1.0), ("BR", -3.0), ("IN", 5.5), ("ID", 7.0)]
NAMES = {"US": "the United States", "DE": "Germany", "BR": "Brazil", "IN": "India", "ID": "Indonesia"}
KINDS = ["yes", "maybe", "unknown", "no"]
FAMILIES = ["himalayas", "jobicy", "remoteok", "weworkremotely", "workable", "ycombinator", "greenhouse", "ashby", "lever"]

# What each board's fields mean, as the board documents them: what a person
# reading the listing there would know. Not loker's reading of them.
BOARD = {
    "himalayas": "Himalayas lists remote jobs only. An empty locationRestrictions means the employer hires from "
                 "anywhere; timezoneRestrictions lists the UTC offsets allowed (empty means no restriction).",
    "jobicy": "Jobicy lists remote jobs only; jobGeo is where the hire may be based.",
    "remoteok": "Remote OK lists remote jobs only; location is where the hire may be based, as the employer typed it.",
    "weworkremotely": "We Work Remotely lists remote jobs only; region is where the hire may be based.",
    "workable": "Workable's job search across the companies hiring through Workable; workplace is remote, hybrid "
                "or on_site and locations are the places, both as the employer set them.",
    "ycombinator": "Y Combinator's job board for its companies; location is as the company set it, with remote "
                   "options shown as \"Remote\" or \"Remote (...)\". The posting's description is not included.",
    "greenhouse": "An employer's own job board (Greenhouse); it lists on-site, hybrid and remote jobs.",
    "ashby": "An employer's own job board (Ashby); it lists on-site, hybrid and remote jobs.",
    "lever": "An employer's own job board (Lever); it lists on-site, hybrid and remote jobs.",
}
DROP = re.compile(r"(?i)url|logo|link|slug|guid|^id$|_id$|date|epoch|compliance|^ai_|metadata|compensation|salary|"
                  r"apply|excerpt|content|description|^lists$|^text$|^title$|^position$|^jobTitle$|tags|categories$|"
                  r"departments|^data_|^company|^internal|^expir|^published|^created|^first_published|^language")


def load(name, default):
    p = os.path.join(HERE, name)
    return json.load(open(p)) if os.path.exists(p) else default


def save(name, obj):
    with open(os.path.join(HERE, name), "w") as f:
        json.dump(obj, f, indent=1, sort_keys=True, ensure_ascii=False)


def arg(flag, default):
    return sys.argv[sys.argv.index(flag) + 1] if flag in sys.argv else default


SET = arg("--set", "sample")


def family(source_id):
    return source_id.split(":")[0]


def connect(db):
    return sqlite3.connect(f"file:{db}?mode=ro", uri=True)


def jobs(dbs):
    """Active canonical jobs: id -> title, company, description, listings."""
    out = {}
    for db in [dbs] if isinstance(dbs, str) else dbs:
        out.update(jobs_in(db))
    return out


def jobs_in(db):
    c = connect(db)
    out = {}
    for jid, job in c.execute("SELECT id, job FROM canonical_jobs WHERE status = 'active' ORDER BY id"):
        j = json.loads(job)
        out[jid] = {"title": j["title"], "company": j["company"], "description": j.get("descriptionMd") or "",
                    "stored": j.get("eligibility"), "listings": []}
    rows = c.execute("SELECT canonical_job_id, source_id, source_kind, source_job_id, raw_data, job, status "
                     "FROM source_jobs ORDER BY source_kind DESC, source_id")
    for cid, sid, kind, sjid, raw, job, status in rows:
        if cid in out:
            out[cid]["listings"].append({"source": sid, "kind": kind, "sourceJobId": sjid, "raw": json.loads(raw),
                                         "stored": json.loads(job) if job else None, "live": status == "live"})
    return out


def primary(job):
    """The listing the store takes text from: employer boards first, then by id."""
    live = [l for l in job["listings"] if l["live"]] or job["listings"]
    return sorted(live, key=lambda l: (l["kind"] != "employer", l["source"]))[0]


def predicted(ids, js, binary=None):
    lines = "".join(json.dumps({"id": j, "viewers": VIEWERS, "listings": js[j]["listings"]}) + "\n" for j in ids)
    out = subprocess.run([binary or arg("--bin", BIN)], input=lines, capture_output=True, text=True, check=True).stdout
    return {r["id"]: r for r in (json.loads(l) for l in out.split("\n") if l)}


def parity():
    """The rebuilt eligibility should equal what ingest stored, unless the
    rules changed since. Checks the pipeline before anything is scored."""
    db = arg("--db", DBS["sample"][0])
    js = jobs(db)
    got = predicted(sorted(js), js)
    keys = ("scope", "regions", "countries", "onsiteCity", "tz")
    same = sum(all(got[j]["eligibility"].get(k) == (js[j]["stored"] or {}).get(k) for k in keys) for j in got)
    rec = Counter(family(primary(js[j])["source"]) for j in got if got[j]["recovered"])
    print(f"{db}: {len(got)}/{len(js)} rebuilt, same eligibility as stored {same}/{len(got)}; remote flag recovered: {dict(rec)}")
    diff = [j for j in got if any(got[j]["eligibility"].get(k) != (js[j]["stored"] or {}).get(k) for k in keys)]
    for j in diff[:int(arg("--show", "0"))]:
        print(j, {k: (js[j]["stored"] or {}).get(k) for k in keys}, "->", {k: got[j]["eligibility"].get(k) for k in keys})


def draw(js, n, cap, rng, exclude=()):
    """n jobs per source family, at most `cap` per company, in random order."""
    by = defaultdict(list)
    for j in sorted(js):
        if j not in exclude:
            by[family(primary(js[j])["source"])].append(j)
    out = {}
    for f in FAMILIES:
        pool = by.get(f, [])
        rng.shuffle(pool)
        per, picked = Counter(), []
        for j in pool:
            c = js[j]["company"].lower()
            if per[c] < cap:
                per[c] += 1
                picked.append(j)
            if len(picked) == n:
                break
        out[f] = sorted(picked)
        print(f"  {f}: {len(picked)} of {len(pool)} ({len(per)} companies)")
    return out


def sample():
    if os.path.exists(os.path.join(HERE, "sample.json")):
        sys.exit("sets already drawn")
    tune = jobs(DBS["sample"])
    print("sample (tuning):")
    save("sample.json", draw(tune, 28, 28, random.Random("elig-sample")))
    # Holdout: jobs the tuning snapshot doesn't have, by id or URL, and not
    # in the golden set.
    fresh = jobs(DBS["holdout"])
    urls = {l["raw"].get("absolute_url") or l["raw"].get("url") or l["raw"].get("link") or l["raw"].get("hostedUrl")
            for j in tune.values() for l in j["listings"]} - {None}
    golden = {i["id"] for i in json.load(open(os.path.join(EVAL, "golden", "items.json")))["items"]}
    old = {j for j, d in fresh.items() if j in tune or j in golden or any(
        (l["raw"].get("absolute_url") or l["raw"].get("url") or l["raw"].get("link") or l["raw"].get("hostedUrl")) in urls
        for l in d["listings"])}
    print(f"holdout ({len(old)} fresh jobs left out as already known):")
    save("holdout.json", draw(fresh, 20, 3, random.Random("elig-holdout"), exclude=old))


def url(listing):
    r = listing["raw"]
    return (r.get("absolute_url") or r.get("url") or r.get("link") or r.get("hostedUrl") or r.get("jobUrl")
            or r.get("guid") or r.get("applicationLink"))


def sample2():
    """holdout2: the description reader was tuned on the tuning snapshot and
    read jobs in eval/role/snapshot-fresh.db, and holdout was scored twice.
    Draw from a later fetch, leaving out every job any snapshot has."""
    if os.path.exists(os.path.join(HERE, "holdout2.json")):
        sys.exit("holdout2 already drawn")
    import glob
    seen_ids, seen_urls = set(), set()
    for db in glob.glob(os.path.join(EVAL, "*", "*.db")):
        if os.path.basename(db) in ("snapshot-h2.db", "snapshot-agg.db", "snapshot-h2-himalayas.db"):
            continue
        try:
            for jid, d in jobs_in(db).items():
                seen_ids.add(jid)
                seen_urls.update(url(l) for l in d["listings"])
        except sqlite3.Error:
            pass  # an older layout (eval/search, eval/agent): ids only
    seen_urls.discard(None)
    new = jobs(DBS["holdout2"])
    old = {j for j, d in new.items() if j in seen_ids or any(url(l) in seen_urls for l in d["listings"])}
    old |= set(set_ids("sample")) | set(set_ids("holdout"))
    print(f"holdout2 ({len(old)} of {len(new)} jobs left out as already in a snapshot):")
    save("holdout2.json", draw(new, 20, 3, random.Random("elig-holdout2"), exclude=old))


def sample_workable(name="workable"):
    """workable.json: a new source, measured before it is trusted.
    workable2.json: the same after its rules, from jobs `workable` doesn't have."""
    if os.path.exists(os.path.join(HERE, f"{name}.json")):
        sys.exit(f"{name} already drawn")
    earlier = {"workable": [], "workable2": ["workable"], "workable3": ["workable", "workable2"],
               "workable4": ["workable", "workable2", "workable3"]}[name]
    before = {j: d for e in earlier for j, d in jobs(DBS[e]).items()}
    seen = set(before)
    seen_urls = {url(l) for d in before.values() for l in d["listings"]} - {None}
    pool = jobs(DBS[name])
    old = seen | {j for j, d in pool.items() if any(url(l) in seen_urls for l in d["listings"])}
    print(f"{name} ({len(old & set(pool))} of {len(pool)} left out as already in {earlier or 'no earlier set'}):")
    save(f"{name}.json", draw(pool, 60, 3, random.Random(f"elig-{name}"), exclude=old))


def sample_yc():
    if os.path.exists(os.path.join(HERE, "yc.json")):
        sys.exit("yc already drawn")
    seen = jobs(os.path.join(HERE, "snapshot-yc.db"))
    seen_urls = {url(l) for d in seen.values() for l in d["listings"]} - {None}
    pool = jobs(DBS["yc"])
    old = set(seen) | {j for j, d in pool.items() if any(url(l) in seen_urls for l in d["listings"])}
    print(f"yc ({len(old & set(pool))} of {len(pool)} left out as read while writing the parser):")
    save("yc.json", draw(pool, 60, 3, random.Random("elig-yc"), exclude=old))


def set_ids(name=None):
    s = load(f"{name or SET}.json", {})
    return [j for f in FAMILIES for j in s.get(f, [])]


# ---------- label ----------

PROMPT = """You read who can take a job, exactly as the posting states it.

For each job, and for each of these viewers, say whether that person can take the job:
{viewers}
Each viewer lives in that country, at that UTC offset, and wants to work remotely from home there. They will not relocate, and will not commute to an office, even one in their own country.

Answer one of:
- "yes": the posting says the job can be done remotely from the viewer's country: the country is listed, is inside a listed region, or the job is open anywhere; and no required working hours rule out the viewer's offset.
- "maybe": the viewer's country is allowed, but something is unsettled: the posting doesn't say whether the job is remote (it could be on-site there), the viewer's offset is outside hours that are preferred but not required, or the posting hedges ("we may consider candidates in ...").
- "no": the posting rules the viewer out: remote only from other places; on-site or hybrid at an office (even one in the viewer's country); required working hours the viewer's offset can't meet; or the hire must live in, or be authorized to work in, a country other than the viewer's.
- "unknown": the posting doesn't say where the hire may live or work from.

Rules:
- Use everything: the board's fields and the description. When they disagree, the more specific statement wins (the board says anywhere, the description says "US only": only the US is yes).
- A region means its usual members: EMEA is Europe, the Middle East and Africa; APAC is Asia and the Pacific; LATAM is Latin America (Brazil included); North America is the US, Canada and Mexico.
- An office address or a city alone, with nothing saying remote, is not remote: answer "maybe" for a viewer in that country (it could still be remote) and "no" for the others, unless the posting settles it.
- Visa sponsorship, citizenship, clearance and pay don't change the answer, except "must be authorized to work in X" (that is where the hire must be).
- Evidence: the words you read it from, quoted, or "".

Jobs:
{jobs}"""

SCHEMA = {"type": "object", "required": ["labels"], "properties": {"labels": {"type": "array", "items": {
    "type": "object", "required": ["i", "verdicts", "evidence"], "properties": {
        "i": {"type": "integer"},
        "verdicts": {"type": "object", "required": [c for c, _ in VIEWERS],
                     "properties": {c: {"type": "string", "enum": KINDS} for c, _ in VIEWERS}},
        "evidence": {"type": "string"}}}}}}


def board_fields(raw):
    def strip(v):
        if isinstance(v, dict):
            return {k: strip(x) for k, x in v.items() if not DROP.search(k) and not (isinstance(x, str) and len(x) > 300)}
        if isinstance(v, list):
            return [strip(x) for x in v[:40]]
        return v
    return json.dumps(strip(raw), ensure_ascii=False)


def job_text(j):
    lines = [f"Title: {j['title']}", f"Company: {j['company']}"]
    for l in j["listings"]:
        f = family(l["source"])
        lines.append(f"Listed on {l['source']}. {BOARD[f]}\nBoard fields: {board_fields(l['raw'])}")
    lines.append(f"Description: {' '.join(j['description'].split())[:MAX_CHARS]}")
    return "\n".join(lines)


def ask_opus(prompt, n):
    cmd = ["claude", "-p", "--model", JUDGES["opus"], "--tools", "", "--strict-mcp-config",
           "--no-session-persistence", "--setting-sources", "", "--output-format", "json",
           "--system-prompt", "You read job eligibility from job postings. Reply only in the requested JSON.",
           "--json-schema", json.dumps(SCHEMA)]
    err = None
    with tempfile.TemporaryDirectory(prefix="loker-judge-") as cwd:  # no CLAUDE.md
        for _ in range(3):
            try:
                p = subprocess.run(cmd, input=prompt, capture_output=True, text=True, cwd=cwd, timeout=900)
                labels = {l["i"]: l for l in json.loads(p.stdout)["structured_output"]["labels"]}
                assert set(labels) == set(range(n)), sorted(labels)
                return [labels[i] for i in range(n)]
            except Exception as e:  # noqa: BLE001
                err = e
    raise RuntimeError(f"opus failed: {err}")


def ask_deepseek(prompt, n):
    shape = {"labels": [{"i": 0, "verdicts": {c: "no" for c, _ in VIEWERS}, "evidence": "..."}]}
    body = json.dumps({
        "model": JUDGES["deepseek"], "temperature": 0, "response_format": {"type": "json_object"},
        "messages": [{"role": "user", "content": prompt + "\n\nReply with JSON only, shaped as: " + json.dumps(shape)}],
    }).encode()
    req = urllib.request.Request("https://api.deepseek.com/chat/completions", data=body, headers={
        "Content-Type": "application/json", "Authorization": "Bearer " + os.environ["DEEPSEEK_API_KEY"]})
    err = None
    for attempt in range(5):
        if attempt:
            time.sleep(10 * attempt)  # a 429 needs time, not a quick retry
        try:
            with urllib.request.urlopen(req, timeout=300) as r:
                out = json.loads(json.load(r)["choices"][0]["message"]["content"])
            labels = {l["i"]: l for l in out["labels"]}
            assert set(labels) == set(range(n)), sorted(labels)
            assert all(l["verdicts"].get(c) in KINDS for l in labels.values() for c, _ in VIEWERS)
            return [labels[i] for i in range(n)]
        except Exception as e:  # noqa: BLE001
            err = e
    raise RuntimeError(f"deepseek failed: {err}")


def ask_pi(prompt, n):
    """DeepSeek v4.1 through Pi (OpenCode Go): no tools, no session, an empty
    working directory, so nothing but the prompt reaches it. Pi holds the key."""
    shape = {"labels": [{"i": 0, "verdicts": {c: "no" for c, _ in VIEWERS}, "evidence": "..."}]}
    cmd = ["pi", "-p", "--no-tools", "--no-session", "--provider", "opencode-go", "--model", "deepseek-v4.1-flash",
           "--thinking", "high", "--system-prompt",
           "You read job eligibility from job postings. Reply only with the requested JSON, no prose."]
    err = None
    with tempfile.TemporaryDirectory(prefix="loker-judge-") as cwd:
        for attempt in range(3):
            if attempt:
                time.sleep(10 * attempt)
            try:
                p = subprocess.run(cmd, input=prompt + "\n\nReply with JSON only, shaped as: " + json.dumps(shape),
                                   capture_output=True, text=True, cwd=cwd, timeout=900,
                                   env={**os.environ, "SECRETSPEC_REASON": "loker eligibility eval labels"})
                text = p.stdout
                out = json.loads(text[text.index("{"):text.rindex("}") + 1])
                labels = {l["i"]: l for l in out["labels"]}
                assert set(labels) == set(range(n)), sorted(labels)
                assert all(l["verdicts"].get(c) in KINDS for l in labels.values() for c, _ in VIEWERS)
                return [labels[i] for i in range(n)]
            except Exception as e:  # noqa: BLE001
                err = e
    raise RuntimeError(f"pi deepseek-v4.1 failed: {err}")


def key(jid):
    return f"{PROMPT_VERSION}|{jid}"


def label():
    judge = os.environ.get("JUDGE", "opus")
    labels = load(f"labels-{judge}.json", {})
    todo = [j for j in set_ids() if key(j) not in labels]
    print(f"judge={judge} set={SET}: {len(todo)} to label ({len(labels)} cached)", flush=True)
    js = jobs(DBS[SET])
    viewers = "\n".join(f"- {c}: lives in {NAMES[c]}, UTC{tz:+g}" for c, tz in VIEWERS)
    batches = [todo[i:i + 4] for i in range(0, len(todo), 4)]

    def one(batch):
        text = "\n---\n".join(f"[{n}] {job_text(js[j])}" for n, j in enumerate(batch))
        ask = {"opus": ask_opus, "deepseek": ask_deepseek, "deepseek41": ask_pi}[judge]
        return list(zip(batch, ask(PROMPT.format(viewers=viewers, jobs=text), len(batch))))

    done = 0
    workers = int(os.environ.get("LABEL_WORKERS", 4 if judge == "opus" else 8))
    with ThreadPoolExecutor(workers) as ex:
        for pairs in ex.map(one, batches):
            for jid, l in pairs:
                labels[key(jid)] = {"verdicts": l["verdicts"], "evidence": l.get("evidence", "")}
            done += 1
            if done % 5 == 0:
                save(f"labels-{judge}.json", labels)
                print(f"  {done}/{len(batches)} batches", flush=True)
    save(f"labels-{judge}.json", labels)
    print("done")


def agree():
    other = arg("--with", "deepseek")
    a, b = load("labels-opus.json", {}), load(f"labels-{other}.json", {})
    ids = [j for j in set_ids() if key(j) in a and key(j) in b]
    pairs = [(a[key(j)]["verdicts"][c], b[key(j)]["verdicts"][c]) for j in ids for c, _ in VIEWERS]
    print(f"{SET}: {len(ids)} jobs x {len(VIEWERS)} viewers labelled by both, same answer {sum(x == y for x, y in pairs)}/{len(pairs)}")
    print(f"   (opus, {other}):", dict(Counter(p for p in pairs if p[0] != p[1]).most_common()))


# ---------- score ----------

def score():
    judge = os.environ.get("JUDGE", "opus")
    labels = load(f"labels-{judge}.json", {})
    ids = [j for j in set_ids() if key(j) in labels]
    js = jobs(DBS[SET])
    got = predicted(ids, js)
    rows = [(j, c, labels[key(j)]["verdicts"][c], got[j]["verdicts"][c]["kind"]) for j in ids for c, _ in VIEWERS]
    print(f"{len(ids)} labelled jobs x {len(VIEWERS)} viewers in {SET}.json (judge={judge})")
    # A "yes" sends someone to a job; a wrong one (they can't take it) is the
    # number to keep at zero. A "no" turns them away. "maybe" and "unknown"
    # make no claim; how often they could have been yes or no is coverage.
    for ours in KINDS:
        mine = [r for r in rows if r[3] == ours]
        print(f"  loker {ours:8} {len(mine):4}   labels: {dict(Counter(r[2] for r in mine).most_common())}")
    yes = [r for r in rows if r[3] == "yes"]
    no = [r for r in rows if r[3] == "no"]
    print(f"  yes precision {sum(r[2] == 'yes' for r in yes)}/{len(yes)}   wrong yes (label no) {sum(r[2] == 'no' for r in yes)}")
    print(f"  no  precision {sum(r[2] == 'no' for r in no)}/{len(no)}   wrong no (label yes) {sum(r[2] == 'yes' for r in no)}")
    stated = [r for r in rows if r[2] in ("yes", "no")]
    print(f"  stated yes/no found {sum(r[2] == r[3] for r in stated)}/{len(stated)}")
    print("  by family (wrong yes / yes, wrong no / no):")
    for f in FAMILIES:
        fr = [r for r in rows if family(primary(js[r[0]])["source"]) == f]
        fy, fn = [r for r in fr if r[3] == "yes"], [r for r in fr if r[3] == "no"]
        print(f"    {f:15} {sum(r[2] != 'yes' for r in fy):3}/{len(fy):<4} {sum(r[2] == 'yes' for r in fn):3}/{len(fn):<4}")
    if "-v" in sys.argv and SET not in ("holdout", "holdout2", "workable4"):
        bad = sorted({r[0] for r in rows if (r[3] == "yes" and r[2] != "yes") or (r[3] == "no" and r[2] == "yes")})
        for j in bad:
            print(f"\n{j} [{primary(js[j])['source']}] {js[j]['title'][:60]}")
            print("  label:", labels[key(j)]["verdicts"], "|", labels[key(j)]["evidence"][:240])
            print("  ours: ", {c: v["kind"] for c, v in got[j]["verdicts"].items()}, "|",
                  next(iter(got[j]["verdicts"].values()))["reason"][:120])


if __name__ == "__main__":
    cmds = {"parity": parity, "sample": sample, "sample2": sample2, "sample-workable": sample_workable, "sample-workable2": lambda: sample_workable("workable2"),
            "sample-workable3": lambda: sample_workable("workable3"),
            "sample-workable4": lambda: sample_workable("workable4"),
            "sample-yc": sample_yc, "label": label, "agree": agree, "score": score}
    if len(sys.argv) < 2 or sys.argv[1] not in cmds:
        sys.exit(__doc__)
    cmds[sys.argv[1]]()
