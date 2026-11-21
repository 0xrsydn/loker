"""Pay-from-description eval for loker: `normalize::pay_from_text` against pay
labelled by Opus (and DeepSeek as a second labeler). Run by hand; not part of
CI. See README.md.

  python3 pay_eval.py sample [n]                    # pick jobs from snapshot.db -> sample.json
  python3 pay_eval.py sample 150 --set holdout2     # a fresh holdout -> holdout2.json
  JUDGE=opus|deepseek python3 pay_eval.py label     # label unlabelled sample jobs (cached)
  python3 pay_eval.py agree                         # Opus vs DeepSeek
  python3 pay_eval.py score [--bin PATH] [-v]       # the rules vs the labels
  (label and score take --set NAME to use NAME.json; --holdout is --set holdout)

A new set never includes a job in another set or in seen.json, the jobs whose
text was looked at while writing the rules. Add to seen.json whenever you read
a job's description to change the rules, so a later holdout stays unseen.

The rules run through `examples/pay_text.rs` (build it with
`cargo build --release -p loker-core --example pay_text`), which reads each
job's description and prints what it would store.
"""
import json, os, random, re, sqlite3, subprocess, sys, tempfile, urllib.request
from collections import Counter, defaultdict
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
DB = os.environ.get("LOKER_EVAL_DB", os.path.join(HERE, "snapshot.db"))
BIN = os.path.join(REPO, "target", "release", "examples", "pay_text")
PROMPT_VERSION = "v1"
JUDGES = {"opus": "claude-opus-5-5", "deepseek": "deepseek-v4-pro"}
MAX_CHARS = 15000

MENTION = re.compile(
    r"([$€£₹]|\b(USD|EUR|GBP|CAD|AUD|INR|IDR|SGD|PHP|CHF|JPY|BRL|MXN|PLN|SEK|NOK|DKK)\b)\s?\d"
    r"|\d[\d,.]*\s?(k|K)?\s?\b(USD|EUR|GBP|CAD|AUD|INR|IDR|SGD|PHP|CHF)\b")


def load(name, default):
    p = os.path.join(HERE, name)
    return json.load(open(p)) if os.path.exists(p) else default


def save(name, obj):
    with open(os.path.join(HERE, name), "w") as f:
        json.dump(obj, f, indent=1, sort_keys=True, ensure_ascii=False)


def arg(flag, default):
    return sys.argv[sys.argv.index(flag) + 1] if flag in sys.argv else default


def descriptions(ids=None):
    db = sqlite3.connect(f"file:{DB}?mode=ro", uri=True)
    rows = db.execute("SELECT id, job FROM canonical_jobs WHERE status = 'active' ORDER BY id")
    out = {}
    for jid, job in rows:
        if ids is None or jid in ids:
            j = json.loads(job)
            out[jid] = {"title": j["title"], "company": j["company"],
                        "description": j.get("descriptionMd") or "", "sourcePay": j.get("pay")}
    return out


SET = arg("--set", "holdout" if "--holdout" in sys.argv else "sample")
SAMPLE = f"{SET}.json"


def sample():
    n = int(sys.argv[2]) if len(sys.argv) > 2 and sys.argv[2].isdigit() else 300
    taken = set(load("seen.json", []))
    for name in os.listdir(HERE):
        if name.startswith(("sample", "holdout")) and name.endswith(".json") and name != SAMPLE:
            taken |= {j for ids in load(name, {}).values() for j in ids}
    jobs = {j: d for j, d in descriptions().items() if j not in taken}
    mention = sorted(j for j, d in jobs.items() if MENTION.search(d["description"].replace("\\$", "$")))
    rest = sorted(set(jobs) - set(mention))
    rng = random.Random({"sample": 1, "holdout": 2}.get(SET, SET))
    a = rng.sample(mention, min(n * 2 // 3, len(mention)))
    b = rng.sample(rest, min(n - len(a), len(rest)))
    save(SAMPLE, {"mention": sorted(a), "rest": sorted(b)})
    print(f"{len(a)} of {len(mention)} jobs mentioning money, {len(b)} of {len(rest)} others -> {SAMPLE}")


def sample_ids_of(name):
    s = load(name, {})
    return s.get("mention", []) + s.get("rest", [])


def sample_ids():
    s = load(SAMPLE, {})
    return s.get("mention", []) + s.get("rest", [])


PROMPT = """You extract the pay a job posting offers, exactly as the posting states it.

For each job, give:
- base: the salary, wage, stipend or contract rate for this job. One entry per range; several only when the posting gives different ranges by location or level, each with its label ("US", "Canada", "Senior"). Empty when the posting states no base pay.
- extras: pay on top of base that the posting offers this hire: equity, bonus, commission, ote (on-target earnings), sign_on. With amount or percent when stated, else both null.
- evidence: the sentence(s) you read pay from, quoted, or "".

Rules:
- Only pay for this job. Not company funding or valuation, revenue, sales targets or quotas, budgets, grant sizes, product prices, metrics, or what the hire will manage ("administer equity plans" is not equity for the hire).
- "Competitive salary", "DOE", "market rate" and benefits (health insurance, PTO, 401k) are not pay.
- One figure: min = max. "Up to X": max only (min null). "From X" or "X+": min only.
- Write numbers in full: 120k -> 120000, 1.2M -> 1200000, 15 LPA -> 1500000 INR a year.
- currency: ISO 4217. "$" alone is USD unless the posting makes it another dollar (CA$, A$, "CAD").
- period: hour, day, week, month or year, as stated or unmistakably implied (a "$120,000 - $150,000 base salary" is a year). If you can't tell, leave that range out.
- On-target earnings go in extras as ote, not base. A base stated separately goes in base.
- Equity as a percentage of the company goes in percent; a bonus as a percentage of base goes in percent.

Jobs:
{jobs}"""

MONEY = {"type": ["object", "null"], "required": ["min", "max", "currency", "period"], "properties": {
    "min": {"type": ["number", "null"]}, "max": {"type": ["number", "null"]},
    "currency": {"type": "string"},
    "period": {"type": ["string", "null"], "enum": ["hour", "day", "week", "month", "year", None]}}}
SCHEMA = {"type": "object", "required": ["labels"], "properties": {"labels": {"type": "array", "items": {
    "type": "object", "required": ["i", "base", "extras", "evidence"], "properties": {
        "i": {"type": "integer"},
        "base": {"type": "array", "items": {
            "type": "object", "required": ["min", "max", "currency", "period", "label"], "properties": {
                **MONEY["properties"], "label": {"type": ["string", "null"]}}}},
        "extras": {"type": "array", "items": {
            "type": "object", "required": ["kind", "amount", "percent"], "properties": {
                "kind": {"type": "string", "enum": ["equity", "bonus", "commission", "ote", "sign_on"]},
                "amount": MONEY,
                "percent": {"type": ["object", "null"], "required": ["min", "max"], "properties": {
                    "min": {"type": ["number", "null"]}, "max": {"type": ["number", "null"]}}}}}},
        "evidence": {"type": "string"}}}}}}


def ask_opus(prompt, n):
    cmd = ["claude", "-p", "--model", JUDGES["opus"], "--tools", "", "--strict-mcp-config",
           "--no-session-persistence", "--setting-sources", "", "--output-format", "json",
           "--system-prompt", "You extract pay from job postings. Reply only in the requested JSON.",
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
    shape = {"labels": [{"i": 0, "base": [{"min": 120000, "max": 150000, "currency": "USD", "period": "year", "label": None}],
                         "extras": [{"kind": "equity", "amount": None, "percent": {"min": 0.1, "max": 0.5}}],
                         "evidence": "..."}]}
    body = json.dumps({
        "model": JUDGES["deepseek"], "temperature": 0, "response_format": {"type": "json_object"},
        "messages": [{"role": "user", "content": prompt + "\n\nReply with JSON only, shaped as: " + json.dumps(shape)}],
    }).encode()
    req = urllib.request.Request("https://api.deepseek.com/chat/completions", data=body, headers={
        "Content-Type": "application/json", "Authorization": "Bearer " + os.environ["DEEPSEEK_API_KEY"]})
    err = None
    for _ in range(3):
        try:
            with urllib.request.urlopen(req, timeout=300) as r:
                out = json.loads(json.load(r)["choices"][0]["message"]["content"])
            labels = {l["i"]: l for l in out["labels"]}
            assert set(labels) == set(range(n)), sorted(labels)
            for l in labels.values():
                l.setdefault("base", []); l.setdefault("extras", []); l.setdefault("evidence", "")
            return [labels[i] for i in range(n)]
        except Exception as e:  # noqa: BLE001
            err = e
    raise RuntimeError(f"deepseek failed: {err}")


def label():
    judge = os.environ.get("JUDGE", "opus")
    labels = load(f"labels-{judge}.json", {})
    key = lambda jid: f"{PROMPT_VERSION}|{jid}"
    todo = [j for j in sample_ids() if key(j) not in labels]
    print(f"judge={judge}: {len(todo)} to label ({len(labels)} cached)", flush=True)
    texts = descriptions(set(todo))
    batches = [todo[i:i + 5] for i in range(0, len(todo), 5)]

    def one(batch):
        jobs = "\n---\n".join(
            f"[{n}] Title: {texts[j]['title']}\nCompany: {texts[j]['company']}\n"
            f"Description: {' '.join(texts[j]['description'].split())[:MAX_CHARS]}"
            for n, j in enumerate(batch))
        ask = ask_opus if judge == "opus" else ask_deepseek
        return list(zip(batch, ask(PROMPT.format(jobs=jobs), len(batch))))

    done = 0
    with ThreadPoolExecutor(4 if judge == "opus" else 8) as ex:
        for pairs in ex.map(one, batches):
            for jid, l in pairs:
                labels[key(jid)] = {k: l[k] for k in ("base", "extras", "evidence")}
            done += 1
            if done % 5 == 0:
                save(f"labels-{judge}.json", labels)
                print(f"  {done}/{len(batches)} batches", flush=True)
    save(f"labels-{judge}.json", labels)
    print("done")


def close(a, b):
    if a is None or b is None:
        return a is None and b is None
    return abs(a - b) <= 0.01 * max(abs(a), abs(b))


def same_money(a, b):
    return (a["currency"].upper() == b["currency"].upper() and a.get("period") == b.get("period")
            and close(a.get("min"), b.get("min")) and close(a.get("max"), b.get("max")))


def compare(gold, got):
    gb, pb = gold.get("base", []), got.get("base", []) if got else []
    free = list(gb)
    for p in pb:
        g = next((g for g in free if same_money(p, g)), None)
        if g is not None:
            free.remove(g)
    right = found = len(gb) - len(free)
    gk = {e["kind"]: e for e in gold.get("extras", [])}
    pk = {e["kind"]: e for e in (got.get("extras", []) if got else [])}
    return {"gold": len(gb), "pred": len(pb), "right": right, "found": found,
            "x_gold": len(gk), "x_pred": len(pk), "x_right": len(set(gk) & set(pk))}


def agree():
    a, b = load("labels-opus.json", {}), load("labels-deepseek.json", {})
    keys = [k for k in a if k in b]
    c = Counter()
    diffs = []
    for k in keys:
        r = compare(a[k], b[k])
        has_a, has_b = bool(a[k]["base"]), bool(b[k]["base"])
        c["both_none"] += not has_a and not has_b
        c["one_none"] += has_a != has_b
        same = r["right"] == r["pred"] == r["found"] == r["gold"]
        c["same_base"] += same
        c["same_extras"] += r["x_right"] == r["x_gold"] == r["x_pred"]
        if not same:
            diffs.append((k, a[k]["base"], b[k]["base"]))
    n = len(keys)
    print(f"{n} jobs labelled by both: same base {c['same_base'] / n:.0%} "
          f"(both none {c['both_none']}, one says none {c['one_none']}), same extra kinds {c['same_extras'] / n:.0%}")
    for k, x, y in diffs[: 20 if "-v" in sys.argv else 8]:
        print(f"  {k}\n    opus     {x}\n    deepseek {y}")


def predicted(binary, ids):
    texts = descriptions(set(ids))
    lines = "".join(json.dumps({"id": j, "text": texts[j]["description"]}) + "\n" for j in ids)
    out = subprocess.run([binary], input=lines, capture_output=True, text=True, check=True).stdout
    return {r["id"]: r["pay"] for r in (json.loads(l) for l in out.split("\n") if l)}


def templates(desc):
    t = desc.replace("\\$", "$")
    return {re.sub(r"\d", "9", t[max(0, m.start() - 30):m.start()]) for m in re.finditer(r"[$€£₱]\s?\d", t)}


def score():
    binary = os.path.abspath(arg("--bin", BIN))
    judge = os.environ.get("JUDGE", "opus")
    labels = load(f"labels-{judge}.json", {})
    strata = load(SAMPLE, {})
    ids = [j for j in sample_ids() if f"{PROMPT_VERSION}|{j}" in labels]
    gold = {j: labels[f"{PROMPT_VERSION}|{j}"] for j in ids}
    got = predicted(binary, ids)
    texts = descriptions(set(ids))
    tuning = sample_ids_of("sample.json")
    tk = set().union(*(templates(d["description"]) for d in descriptions(set(tuning)).values()))
    novel = {j for j in ids if not (templates(texts[j]["description"]) & tk)}
    print(f"{len(ids)} labelled jobs in {SAMPLE} (judge={judge}, {os.path.basename(binary)})")
    for name in ("mention", "rest", "all", "novel"):
        if name == "novel" and SAMPLE == "sample.json":
            continue
        sel = [j for j in ids if name == "all" or (name == "novel" and j in novel) or j in strata.get(name, [])]
        c = Counter()
        for j in sel:
            r = compare(gold[j], got.get(j))
            for k, v in r.items():
                c[k] += v
            g, p = bool(gold[j]["base"]), bool(got.get(j) and got[j]["base"])
            c["jobs_gold"] += g
            c["jobs_pred"] += p
            c["jobs_exact"] += g and r["right"] == r["pred"] == r["found"] == r["gold"]
            c["jobs_wrong"] += p and r["right"] < r["pred"]
        pct = lambda a, b: f"{a / b:.0%}" if b else "-"
        print(f"\n  {name}: {len(sel)} jobs, {c['jobs_gold']} state base pay")
        print(f"    base ranges   precision {pct(c['right'], c['pred'])} ({c['right']}/{c['pred']})   "
              f"recall {pct(c['found'], c['gold'])} ({c['found']}/{c['gold']})")
        print(f"    jobs          exact {pct(c['jobs_exact'], c['jobs_gold'])} of those with pay   "
              f"with a wrong figure {c['jobs_wrong']} of {c['jobs_pred']} given pay")
        print(f"    extra kinds   precision {pct(c['x_right'], c['x_pred'])}   recall {pct(c['x_right'], c['x_gold'])}")
    wrong, missed = [], []
    for j in ids:
        r = compare(gold[j], got.get(j))
        if r["right"] < r["pred"]:
            wrong.append(j)
        elif r["found"] < r["gold"]:
            missed.append(j)
    k = 30 if "-v" in sys.argv else 10
    short = lambda p: [(m["currency"], m.get("period"), m.get("min"), m.get("max")) for m in (p or {}).get("base", [])]
    print(f"\n  wrong figures ({len(wrong)}):")
    for j in wrong[:k]:
        print(f"    {j} {texts[j]['title'][:50]!r}\n      gold {short(gold[j])}\n      ours {short(got.get(j))}  from {(got.get(j) or {}).get('raw', '')[:140]!r}")
    print(f"\n  missed ({len(missed)}):")
    for j in missed[:k]:
        print(f"    {j} {texts[j]['title'][:50]!r} gold {short(gold[j])}  ours {short(got.get(j))}\n      {gold[j]['evidence'][:160]!r}")


if __name__ == "__main__":
    cmds = {"sample": sample, "label": label, "score": score, "agree": agree}
    if len(sys.argv) < 2 or sys.argv[1] not in cmds:
        sys.exit(__doc__)
    cmds[sys.argv[1]]()
