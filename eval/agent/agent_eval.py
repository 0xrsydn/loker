"""Agent eval for loker: an LLM agent answers job-hunting tasks over MCP.
Run by hand; not part of CI. See README.md.

  python3 agent_eval.py run [--label L] [--trials 3] [--bin PATH] [--only TASK,...]
  python3 agent_eval.py report [LABEL ...]

For each task, the ground truth is every job the viewer can apply to (verdict
yes or maybe, listed by the binary under test with no query, so search
ranking can't bias it) that Opus grades as fitting the role. Then Pi
(DeepSeek Flash, only loker's MCP tools) runs each task on a fresh copy of
the search eval's snapshot. The agent ends with a `PICKS:` line of job ids;
picks are scored against the truth, and tool calls, characters of tool
output, tokens, cost and time are recorded.

Needs DEEPSEEK_API_KEY in the environment (Pi) and the `claude` CLI (judge).
"""
import argparse, glob, json, os, re, shutil, sqlite3, subprocess, sys, tempfile, time
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
SNAPSHOT = os.environ.get("LOKER_EVAL_DB", os.path.join(REPO, "eval", "search", "snapshot.db"))
RUNS = os.path.join(HERE, "runs")
LABELS = os.path.join(HERE, "fit-labels.json")
MODEL = ("deepseek", "deepseek-flash")
TIMEOUT = 900
PICKS_LINE = ("\n\nFinish with one line: `PICKS:` followed by the ids of the jobs you recommend, "
              "comma-separated, or `PICKS: none`.")
JOB_ID = re.compile(r"\bj_[0-9a-f]{10}\b")

sys.path.insert(0, os.path.join(REPO, "eval", "search"))


def tasks(only=None):
    ts = json.load(open(os.path.join(HERE, "tasks.json")))
    return [t for t in ts if not only or t["id"] in only]


def label_default():
    sha = subprocess.run(["git", "-C", REPO, "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip()
    dirty = subprocess.run(["git", "-C", REPO, "status", "--porcelain", "--", "crates"], capture_output=True, text=True).stdout.strip()
    return sha + ("-dirty" if dirty else "")


def loker(binary, home, *args):
    out = subprocess.run([binary, *args, "--json"], env={**os.environ, "LOKER_HOME": home},
                         capture_output=True, text=True, check=True).stdout
    return json.loads(out)


def fresh_home():
    work = tempfile.mkdtemp(prefix="loker-agent-")
    home = os.path.join(work, "home")
    os.makedirs(home)
    shutil.copy(SNAPSHOT, os.path.join(home, "loker.db"))
    return work, home


# ---------- ground truth ----------

def eligible(binary, home, task):
    """Every job the task's viewer can apply to: id -> verdict."""
    out, offset = {}, 0
    while True:
        r = loker(binary, home, "search", "--country", task["country"], f"--tz={task['tz']}",
                  "--min-verdict", "maybe", "--compact", "--limit", "100", "--offset", str(offset))
        out.update({j["id"]: j["eligible"]["kind"] for j in r["jobs"]})
        if not r.get("nextOffset"):
            return out
        offset = r["nextOffset"]


def grade(task, ids, labels):
    """Opus grades role fit (0-3) for jobs not yet graded for this task."""
    import eval as search_eval  # eval/search/eval.py: the same judge plumbing
    todo = sorted(j for j in ids if f"{task['id']}|{j}" not in labels)
    db = sqlite3.connect(SNAPSHOT)
    for i in range(0, len(todo), 15):
        chunk = todo[i:i + 15]
        jobs = []
        for n, jid in enumerate(chunk):
            t, c, d = db.execute("SELECT title, company, description_md FROM canonical_jobs WHERE id = ?", (jid,)).fetchone()
            jobs.append(f"[{n}] Title: {t}\nCompany: {c}\nDescription (start):\n{(d or '')[:800]}\n")
        prompt = search_eval.BATCH.format(q=task["role"], intent=task["role"], jobs="\n---\n".join(jobs))
        grades, _ = search_eval.ask_opus(prompt, len(chunk))
        for jid, g in zip(chunk, grades):
            labels[f"{task['id']}|{jid}"] = g
    return labels


def truth(binary, ts):
    """Per task: the eligible jobs that fit the role (grade >= 2)."""
    work, home = fresh_home()
    try:
        labels = json.load(open(LABELS)) if os.path.exists(LABELS) else {}
        out = {}
        for t in ts:
            elig = eligible(binary, home, t)
            labels = grade(t, elig, labels)
            json.dump(labels, open(LABELS, "w"), indent=1, sort_keys=True)
            good = sorted(j for j in elig if labels[f"{t['id']}|{j}"]["grade"] >= 2)
            out[t["id"]] = {"eligible": elig, "good": good}
        return out
    finally:
        shutil.rmtree(work, ignore_errors=True)


# ---------- runs ----------

def run_one(binary, task, trial, out_dir, gt):
    work, home = fresh_home()
    try:
        os.makedirs(os.path.join(work, ".pi"))
        mcp = {"mcpServers": {"loker": {
            "command": binary, "args": ["mcp"],
            "env": {"LOKER_HOME": home, "RUST_LOG": "warn"},
            "exposure": "direct",
            "description": "loker: remote job search with eligibility (who can apply from where)"}}}
        json.dump(mcp, open(os.path.join(work, ".pi", "mcp.json"), "w"))
        sessions = os.path.join(work, "sessions")
        cmd = ["pi", "-p", "-a", "--no-builtin-tools", "--tools", "mcp__loker__*",
               "--provider", MODEL[0], "--model", MODEL[1], "--session-dir", sessions,
               task["prompt"] + PICKS_LINE]
        t0 = time.time()
        try:
            p = subprocess.run(cmd, cwd=work, capture_output=True, text=True, timeout=TIMEOUT,
                               env={**os.environ, "SECRETSPEC_REASON": "loker agent eval"})
            status = "ok" if p.returncode == 0 else f"exit {p.returncode}"
        except subprocess.TimeoutExpired:
            status = "timeout"
        rec = {"task": task["id"], "trial": trial, "status": status, "seconds": round(time.time() - t0, 1)}
        files = sorted(glob.glob(os.path.join(sessions, "*.jsonl")))
        if files:
            rec.update(parse_session(files[-1]))
            shutil.copy(files[-1], os.path.join(out_dir, f"{task['id']}-{trial}.session.jsonl"))
        rec.update(score_picks(binary, home, task, rec.get("answer", ""), gt))
        json.dump(rec, open(os.path.join(out_dir, f"{task['id']}-{trial}.json"), "w"), indent=1)
        return rec
    finally:
        shutil.rmtree(work, ignore_errors=True)


def parse_session(path):
    calls, chars, tokens, cost, answer = [], 0, 0, 0.0, ""
    for line in open(path):
        m = json.loads(line).get("message") or {}
        if m.get("role") == "assistant":
            u = m.get("usage") or {}
            tokens += u.get("totalTokens", 0)
            cost += (u.get("cost") or {}).get("total", 0)
            texts = []
            for c in m.get("content", []):
                if c.get("type") == "toolCall":
                    calls.append({"tool": c["name"].split("__")[-1], "args": c.get("arguments")})
                elif c.get("type") == "text":
                    texts.append(c["text"])
            if texts:
                answer = "\n".join(texts)
        elif m.get("role") == "toolResult":
            chars += sum(len(c.get("text", "")) for c in m.get("content", []) if isinstance(c, dict))
    return {"calls": calls, "tool_chars": chars, "tokens": tokens, "cost": round(cost, 5), "answer": answer}


def score_picks(binary, home, task, answer, gt):
    lines = [l for l in answer.splitlines() if "PICKS:" in l]
    if not lines:
        return {"format": "no PICKS line", "picks": []}
    ids = list(dict.fromkeys(JOB_ID.findall(lines[-1])))
    good, elig = set(gt["good"]), gt["eligible"]
    picks = [{"id": j, "verdict": elig.get(j), "good": j in good} for j in ids]
    unknown = [p["id"] for p in picks if p["verdict"] is None]
    if unknown:  # not eligible: say which verdict it has instead
        r = loker(binary, home, "get", *unknown, "--country", task["country"], f"--tz={task['tz']}")
        got = {j["id"]: (j.get("eligible") or {}).get("kind", "?") for j in r["jobs"]}
        for p in picks:
            if p["verdict"] is None:
                p["verdict"] = got.get(p["id"], "no such job")
    return {"format": "ok", "picks": picks, "available": len(good)}


def run(args):
    binary = os.path.abspath(args.bin)
    label = args.label or label_default()
    out_dir = os.path.join(RUNS, label)
    os.makedirs(out_dir, exist_ok=True)
    ts = tasks(args.only.split(",") if args.only else None)
    gt = truth(binary, ts)
    json.dump({k: {"good": v["good"], "eligible": len(v["eligible"])} for k, v in gt.items()},
              open(os.path.join(out_dir, "truth.json"), "w"), indent=1)
    for t in ts:
        print(f"  {t['id']}: {len(gt[t['id']]['eligible'])} eligible, {len(gt[t['id']]['good'])} fit the role", flush=True)
    jobs = [(t, n) for t in ts for n in range(1, args.trials + 1)]
    print(f"label {label}: {len(jobs)} runs", flush=True)
    with ThreadPoolExecutor(args.parallel) as ex:
        list(ex.map(lambda tn: run_one(binary, tn[0], tn[1], out_dir, gt[tn[0]["id"]]), jobs))
    report_labels([label])


# ---------- report ----------

def summarize(records):
    def mean(xs):
        xs = [x for x in xs if x is not None]
        return sum(xs) / len(xs) if xs else float("nan")

    def score(r):
        picks, avail = r.get("picks", []), r.get("available", 0)
        n_good = sum(p["good"] for p in picks)
        want = min(5, avail)
        return {
            "precision": n_good / len(picks) if picks else (1.0 if avail == 0 else None),
            "recall": n_good / want if want else (1.0 if not picks else 0.0),
            "pass": r["status"] == "ok" and r.get("format") == "ok"
                    and n_good == len(picks) and n_good >= want,
        }
    s = [score(r) for r in records]
    return {
        "runs": len(records),
        "failed": sum(r["status"] != "ok" or r.get("format") != "ok" for r in records),
        "calls": mean([len(r.get("calls", [])) for r in records]),
        "kchars": mean([r.get("tool_chars", 0) / 1000 for r in records]),
        "ktokens": mean([r.get("tokens", 0) / 1000 for r in records]),
        "cost": mean([r.get("cost", 0) for r in records]),
        "seconds": mean([r["seconds"] for r in records]),
        "available": mean([r.get("available") for r in records]),
        "picks": mean([len(r.get("picks", [])) for r in records]),
        "precision": mean([x["precision"] for x in s]),
        "recall": mean([x["recall"] for x in s]),
        "pass": mean([x["pass"] for x in s]),
    }


def report_labels(labels):
    print(f"{'label':16} {'task':12} {'runs':>4} {'fail':>4} {'calls':>6} {'kchars':>7} {'ktok':>6} {'$':>7} "
          f"{'sec':>5} {'avail':>5} {'picks':>5} {'prec':>5} {'rec@5':>5} {'pass':>5}")
    for label in labels:
        recs = [json.load(open(f)) for f in sorted(glob.glob(os.path.join(RUNS, label, "*-[0-9]*.json")))]
        groups = {}
        for r in recs:
            groups.setdefault(r["task"], []).append(r)
        for name, rs in [*sorted(groups.items()), ("ALL", recs)]:
            s = summarize(rs)
            print(f"{label:16} {name:12} {s['runs']:4d} {s['failed']:4d} {s['calls']:6.1f} {s['kchars']:7.1f} "
                  f"{s['ktokens']:6.1f} {s['cost']:7.4f} {s['seconds']:5.0f} {s['available']:5.0f} {s['picks']:5.1f} "
                  f"{s['precision']:5.0%} {s['recall']:5.0%} {s['pass']:5.0%}")


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--label")
    r.add_argument("--trials", type=int, default=3)
    r.add_argument("--parallel", type=int, default=3)
    r.add_argument("--bin", default=os.path.join(REPO, "target", "release", "loker"))
    r.add_argument("--only")
    p = sub.add_parser("report")
    p.add_argument("labels", nargs="*")
    a = ap.parse_args()
    if a.cmd == "run":
        run(a)
    else:
        report_labels(a.labels or sorted(os.listdir(RUNS)))


if __name__ == "__main__":
    main()
