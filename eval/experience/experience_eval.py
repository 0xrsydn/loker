"""Years-of-experience eval for loker: `experience_text::experience_from_text`
against the minimum years labelled by Opus and DeepSeek v4.1. Run by hand;
not part of CI. See README.md.

  python3 experience_eval.py sample               # draw sample.json, holdout.json, holdout2.json, once
  python3 experience_eval.py sample2              # draw holdout3.json, holdout4.json, once
  python3 experience_eval.py sample3              # draw holdout5.json, holdout6.json, once
  JUDGE=opus|deepseek41 python3 experience_eval.py label [--set holdout]
  python3 experience_eval.py agree [--set holdout]
  python3 experience_eval.py score [--set holdout] [--bin PATH] [-v]

The tuning sample is from the role eval's fresh snapshot; the holdouts are
from snapshots whose text no experience rule has been written from: 17
employer boards new to loker (visa's snapshot-boards.db, its holdout3 jobs
left out) and Workable's search (eligibility's snapshot-workable4.db). Never
print a holdout job's text or labels before it is scored.
"""
import json, os, random, re, sqlite3, subprocess, sys, tempfile, time
from collections import Counter
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
EVAL = os.path.dirname(HERE)
REPO = os.path.dirname(EVAL)
DBS = {"sample": os.path.join(EVAL, "role", "snapshot-fresh.db"),
       "holdout": os.path.join(EVAL, "visa", "snapshot-boards.db"),
       "holdout2": os.path.join(EVAL, "eligibility", "snapshot-workable4.db"),
       "holdout3": os.path.join(EVAL, "visa", "snapshot-boards2.db"),
       "holdout4": os.path.join(EVAL, "eligibility", "snapshot-h2.db"),
       "holdout5": os.path.join(EVAL, "eligibility", "snapshot-workable3.db"),
       "holdout6": os.path.join(EVAL, "pay", "snapshot.db")}
SIZES = {"sample": (140, 60, 4), "holdout": (110, 40, 8), "holdout2": (70, 30, 3)}  # net, rest, per company
BIN = os.path.join(REPO, "target", "release", "examples", "experience_text")
PROMPT_VERSION = "v1"
MAX_CHARS = 15000

# The sampler's net: wide on purpose; the random share measures what it misses.
NUM = r"(?:\d{1,2}|one|two|three|four|five|six|seven|eight|nine|ten|twelve|fifteen)"
NET = re.compile(rf"(?i)\b{NUM}\s*(?:\+|plus|or more)?\s*(?:-|–|to)?\s*(?:\d{{1,2}})?\s*\+?\s*years?\b|"
                 r"no (?:prior |previous )?experience|new grad|entry[- ]level|recent graduates?")


def load(name, default):
    p = os.path.join(HERE, name)
    return json.load(open(p)) if os.path.exists(p) else default


def save(name, obj):
    with open(os.path.join(HERE, name), "w") as f:
        json.dump(obj, f, indent=1, sort_keys=True, ensure_ascii=False)


def arg(flag, default):
    return sys.argv[sys.argv.index(flag) + 1] if flag in sys.argv else default


SET = arg("--set", "sample")


def descriptions(db, ids=None):
    rows = sqlite3.connect(f"file:{db}?mode=ro", uri=True).execute(
        "SELECT id, job FROM canonical_jobs WHERE status = 'active' ORDER BY id")
    out = {}
    for jid, job in rows:
        if ids is None or jid in ids:
            j = json.loads(job)
            out[jid] = {"title": j["title"], "company": j["company"], "description": j.get("descriptionMd") or ""}
    return out


def set_ids(name=SET):
    s = load(f"{name}.json", {})
    return s.get("net", []) + s.get("rest", [])


def texts(name=SET):
    return descriptions(DBS[name], set(set_ids(name)))


def sample2():
    """holdout3 and holdout4, drawn after the rules were fixed on the first
    two holdouts (which are tuning since): visa's second list of employer
    boards (snapshot-boards2.db), and eligibility's snapshot-h2.db without
    any company in an earlier set's snapshot."""
    if os.path.exists(os.path.join(HERE, "holdout3.json")):
        sys.exit("holdout3 already drawn")
    seen = set()
    for name in ("sample", "holdout", "holdout2"):
        seen |= {d["company"].lower() for d in descriptions(DBS[name]).values()}
    for name, (n_net, n_rest, cap) in (("holdout3", (70, 30, 6)), ("holdout4", (110, 40, 4))):
        jobs = {j: d for j, d in descriptions(DBS[name]).items()
                if d["description"].strip() and d["company"].lower() not in seen}
        rng = random.Random(f"experience-{name}")
        out = {}
        for stratum, n in (("net", n_net), ("rest", n_rest)):
            pool = sorted(j for j, d in jobs.items() if bool(NET.search(d["description"])) == (stratum == "net"))
            rng.shuffle(pool)
            per, picked = Counter(), []
            for j in pool:
                c = jobs[j]["company"].lower()
                if per[c] < cap:
                    per[c] += 1
                    picked.append(j)
                if len(picked) == n:
                    break
            out[stratum] = sorted(picked)
            print(f"{name} {stratum}: {len(picked)} of {len(pool)}, {len(per)} companies")
        save(f"{name}.json", out)


def sample3():
    """holdout5 and holdout6, drawn after the rules were fixed on holdout3
    and holdout4: Workable's earlier search (snapshot-workable3.db) and the
    pay eval's employer boards (Clera, Mercor, Binance, Palantir), without
    any company in an earlier set's snapshot."""
    if os.path.exists(os.path.join(HERE, "holdout5.json")):
        sys.exit("holdout5 already drawn")
    seen = set()
    for name in ("sample", "holdout", "holdout2", "holdout3", "holdout4"):
        seen |= {d["company"].lower() for d in descriptions(DBS[name]).values()}
    employers = {r[0] for r in sqlite3.connect(f"file:{DBS['holdout6']}?mode=ro", uri=True).execute(
        "SELECT canonical_job_id FROM source_jobs WHERE source_kind = 'employer'")}
    for name, (n_net, n_rest, cap), keep in (("holdout5", (75, 25, 3), None), ("holdout6", (32, 8, 10), employers)):
        jobs = {j: d for j, d in descriptions(DBS[name]).items()
                if d["description"].strip() and d["company"].lower() not in seen and (keep is None or j in keep)}
        rng = random.Random(f"experience-{name}")
        out = {}
        for stratum, n in (("net", n_net), ("rest", n_rest)):
            pool = sorted(j for j, d in jobs.items() if bool(NET.search(d["description"])) == (stratum == "net"))
            rng.shuffle(pool)
            per, picked = Counter(), []
            for j in pool:
                c = jobs[j]["company"].lower()
                if per[c] < cap:
                    per[c] += 1
                    picked.append(j)
                if len(picked) == n:
                    break
            out[stratum] = sorted(picked)
            print(f"{name} {stratum}: {len(picked)} of {len(pool)}, {len(per)} companies")
        save(f"{name}.json", out)


def sample():
    if os.path.exists(os.path.join(HERE, "sample.json")):
        sys.exit("sets already drawn")
    v = json.load(open(os.path.join(EVAL, "visa", "holdout3.json")))
    skip = {"holdout": set(v["mention"] + v["rest"])}
    for name, (n_net, n_rest, cap) in SIZES.items():
        jobs = {j: d for j, d in descriptions(DBS[name]).items()
                if d["description"].strip() and j not in skip.get(name, set())}
        rng = random.Random(f"experience-{name}")
        out = {}
        for stratum, n in (("net", n_net), ("rest", n_rest)):
            pool = sorted(j for j, d in jobs.items() if bool(NET.search(d["description"])) == (stratum == "net"))
            rng.shuffle(pool)
            per, picked = Counter(), []
            for j in pool:
                c = jobs[j]["company"].lower()
                if per[c] < cap:
                    per[c] += 1
                    picked.append(j)
                if len(picked) == n:
                    break
            out[stratum] = sorted(picked)
            print(f"{name} {stratum}: {len(picked)} of {len(pool)}, {len(per)} companies")
        save(f"{name}.json", out)


# ---------- label ----------

PROMPT = """You read how many years of work experience a job posting requires, exactly as the posting states it.

For each job, give:
- min_years: the minimum years of experience required for this role, as an integer; 0 when the posting says no experience is needed ("no experience required", "new graduates welcome", "0+ years"); null when it states no number of years.
- max_years: the upper end, only when the posting gives one ("3-5 years" is 5, "up to 2 years" is 2); else null.
- evidence: the phrase you read it from, quoted, or "".

Rules:
- Required only. Years under "preferred", "nice to have", "bonus", "a plus", "ideally" don't count. If the posting gives a required number and a higher preferred one, use the required one; if it gives only preferred years, null.
- Several required numbers (overall experience and experience with particular skills or tasks): the largest, since the hire must meet all of them.
- Alternatives ("5 years with a bachelor's or 3 with a master's", different levels in one posting): the smallest.
- "At least two years" is 2; "5+ years" is 5. "Several years", "extensive experience", "proven track record" with no number: null.
- Not experience: the company's or team's years ("15 years in business"), years of study, contract length, or how long the hire will stay.

Jobs:
{jobs}"""

SCHEMA = {"type": "object", "required": ["labels"], "properties": {"labels": {"type": "array", "items": {
    "type": "object", "required": ["i", "min_years", "max_years", "evidence"], "properties": {
        "i": {"type": "integer"},
        "min_years": {"type": ["integer", "null"]},
        "max_years": {"type": ["integer", "null"]},
        "evidence": {"type": "string"}}}}}}


def valid(l):
    return all(l.get(k) is None or (isinstance(l[k], int) and 0 <= l[k] <= 40) for k in ("min_years", "max_years"))


def ask_opus(prompt, n):
    cmd = ["claude", "-p", "--model", "claude-opus-5-5", "--tools", "", "--strict-mcp-config",
           "--no-session-persistence", "--setting-sources", "", "--output-format", "json",
           "--system-prompt", "You read required years of experience from job postings. Reply only in the requested JSON.",
           "--json-schema", json.dumps(SCHEMA)]
    err = None
    with tempfile.TemporaryDirectory(prefix="loker-judge-") as cwd:  # no CLAUDE.md
        for _ in range(3):
            try:
                p = subprocess.run(cmd, input=prompt, capture_output=True, text=True, cwd=cwd, timeout=900)
                labels = {l["i"]: l for l in json.loads(p.stdout)["structured_output"]["labels"]}
                assert set(labels) == set(range(n)), sorted(labels)
                assert all(valid(l) for l in labels.values())
                return [labels[i] for i in range(n)]
            except Exception as e:  # noqa: BLE001
                err = e
    raise RuntimeError(f"opus failed: {err}")


def ask_pi(prompt, n):
    """DeepSeek v4.1 through Pi (OpenCode Go): no tools, no session, an empty
    working directory, so nothing but the prompt reaches it. Pi holds the key."""
    shape = {"labels": [{"i": 0, "min_years": 3, "max_years": None, "evidence": "..."}]}
    cmd = ["pi", "-p", "--no-tools", "--no-session", "--provider", "opencode-go", "--model", "deepseek-v4.1-flash",
           "--thinking", "high", "--system-prompt",
           "You read required years of experience from job postings. Reply only with the requested JSON, no prose."]
    err = None
    with tempfile.TemporaryDirectory(prefix="loker-judge-") as cwd:
        for attempt in range(3):
            if attempt:
                time.sleep(10 * attempt)
            try:
                p = subprocess.run(cmd, input=prompt + "\n\nReply with JSON only, shaped as: " + json.dumps(shape),
                                   capture_output=True, text=True, cwd=cwd, timeout=900,
                                   env={**os.environ, "SECRETSPEC_REASON": "loker experience eval labels"})
                text = p.stdout
                out = json.loads(text[text.index("{"):text.rindex("}") + 1])
                labels = {l["i"]: l for l in out["labels"]}
                assert set(labels) == set(range(n)), sorted(labels)
                assert all(valid(l) for l in labels.values())
                return [labels[i] for i in range(n)]
            except Exception as e:  # noqa: BLE001
                err = e
    raise RuntimeError(f"pi deepseek-v4.1 failed: {err}")


def key(jid):
    return f"{PROMPT_VERSION}|{SET}|{jid}"


def label():
    judge = os.environ.get("JUDGE", "opus")
    labels = load(f"labels-{judge}.json", {})
    todo = [j for j in set_ids() if key(j) not in labels]
    print(f"judge={judge} set={SET}: {len(todo)} to label ({len(labels)} cached)", flush=True)
    t = texts()
    batches = [todo[i:i + 5] for i in range(0, len(todo), 5)]

    def one(batch):
        jobs = "\n---\n".join(
            f"[{n}] Title: {t[j]['title']}\nCompany: {t[j]['company']}\n"
            f"Description:\n{t[j]['description'][:MAX_CHARS]}"
            for n, j in enumerate(batch))
        ask = ask_opus if judge == "opus" else ask_pi
        return list(zip(batch, ask(PROMPT.format(jobs=jobs), len(batch))))

    done = 0
    workers = int(os.environ.get("LABEL_WORKERS", 4))
    with ThreadPoolExecutor(workers) as ex:
        for pairs in ex.map(one, batches):
            for jid, l in pairs:
                labels[key(jid)] = {k: l.get(k) for k in ("min_years", "max_years", "evidence")}
            done += 1
            if done % 5 == 0:
                save(f"labels-{judge}.json", labels)
                print(f"  {done}/{len(batches)} batches", flush=True)
    save(f"labels-{judge}.json", labels)
    print("done")


def agree():
    a, b = load("labels-opus.json", {}), load(f"labels-{arg('--with', 'deepseek41')}.json", {})
    ids = [j for j in set_ids() if key(j) in a and key(j) in b]
    same = sum(a[key(j)]["min_years"] == b[key(j)]["min_years"] for j in ids)
    print(f"{SET}: {len(ids)} labelled by both, same min_years {same}/{len(ids)}")
    print("  ", Counter((a[key(j)]["min_years"], b[key(j)]["min_years"]) for j in ids
                       if a[key(j)]["min_years"] != b[key(j)]["min_years"]))


# ---------- score ----------

def predicted(binary, ids, t):
    lines = "".join(json.dumps({"id": j, "text": t[j]["description"]}) + "\n" for j in ids)
    out = subprocess.run([binary], input=lines, capture_output=True, text=True, check=True).stdout
    return {r["id"]: r["experience"] for r in (json.loads(l) for l in out.split("\n") if l)}


def score():
    judge = os.environ.get("JUDGE", "opus")
    labels = load(f"labels-{judge}.json", {})
    ids = [j for j in set_ids() if key(j) in labels]
    t = texts()
    got = predicted(arg("--bin", BIN), ids, t)
    gold = {j: labels[key(j)]["min_years"] for j in ids}
    ours = {j: (got.get(j) or {}).get("minYears") for j in ids}
    print(f"{len(ids)} labelled jobs in {SET}.json (judge={judge})")
    # A number is a claim. Too high hides the job from someone who meets the
    # real bar (or invents a bar where none is stated; a 0 hides no one): the
    # number to keep near zero. Too low only shows a job to someone a little short of it.
    claims = [j for j in ids if ours[j] is not None]
    stated = [j for j in ids if gold[j] is not None]
    right = [j for j in claims if ours[j] == gold[j]]
    high = [j for j in claims if (gold[j] is None and ours[j] > 0) or (gold[j] is not None and ours[j] > gold[j])]
    low = [j for j in claims if gold[j] is not None and ours[j] < gold[j]]
    print(f"  claims right {len(right)}/{len(claims)}   stated found {sum(ours[j] == gold[j] for j in stated)}/{len(stated)}")
    print(f"  too high (would hide wrongly) {len(high)}   too low {len(low)}   "
          f"missed {sum(ours[j] is None for j in stated)}")
    gmax = {j: labels[key(j)]["max_years"] for j in ids}
    omax = {j: (got.get(j) or {}).get("maxYears") for j in ids}
    print(f"  max_years same {sum(gmax[j] == omax[j] for j in ids)}/{len(ids)}")
    if "-v" in sys.argv and (not SET.startswith("holdout") or SET in load("burned.json", [])):
        for j in ids:
            if gold[j] != ours[j]:
                print(f"\n{j} {t[j]['title'][:60]}  label={gold[j]} ours={ours[j]}"
                      f"\n  label: {labels[key(j)]['evidence'][:300]}"
                      f"\n  ours:  {((got.get(j) or {}).get('evidence') or '')[:300]}")


if __name__ == "__main__":
    cmds = {"sample": sample, "sample2": sample2, "sample3": sample3, "label": label, "agree": agree, "score": score}
    if len(sys.argv) < 2 or sys.argv[1] not in cmds:
        sys.exit(__doc__)
    cmds[sys.argv[1]]()
