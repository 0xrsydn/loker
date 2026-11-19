"""Role classifier eval for loker: the classifier against labels from Opus
(and DeepSeek as a second labeler). Run by hand; not part of CI. See README.md.

  python3 role_eval.py sample [n]          # pick n active jobs (default 600) -> sample.json
  JUDGE=opus|deepseek python3 role_eval.py label   # label unlabelled sample jobs (cached)
  python3 role_eval.py score [--bin PATH] [-v]     # classifier vs labels
  python3 role_eval.py agree               # Opus vs DeepSeek
  python3 role_eval.py jev                 # Jev's choice per sample job -> predictions-jev.json (cached)
  python3 role_eval.py score --pred jev|hybrid[:0.6]   # score Jev, or rules with Jev for the rest

The snapshot (snapshot.db, a loker database, local only) is copied for each
run, so the binary under test classifies it on open with its own rules.
"""
import json, os, random, re, shutil, sqlite3, subprocess, sys, tempfile, urllib.request
from collections import Counter, defaultdict
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
DB = os.environ.get("LOKER_EVAL_DB", os.path.join(HERE, "snapshot.db"))
BIN = os.path.join(REPO, "target", "release", "loker")
PROMPT_VERSION = "v1"
JUDGES = {"opus": "claude-opus-5-5", "deepseek": "deepseek-v4-pro"}


def load(name, default):
    p = os.path.join(HERE, name)
    return json.load(open(p)) if os.path.exists(p) else default


def save(name, obj):
    with open(os.path.join(HERE, name), "w") as f:
        json.dump(obj, f, indent=1, sort_keys=True)


def arg(flag, default):
    return sys.argv[sys.argv.index(flag) + 1] if flag in sys.argv else default


def loker(binary, home, *a):
    out = subprocess.run([binary, *a, "--json"], env={**os.environ, "LOKER_HOME": home},
                         capture_output=True, text=True, check=True).stdout
    return json.loads(out)


def with_home(binary, fn):
    home = tempfile.mkdtemp(prefix="loker-role-")
    try:
        shutil.copy(DB, os.path.join(home, "loker.db"))
        return fn(home)
    finally:
        shutil.rmtree(home, ignore_errors=True)


# ---------- sample ----------

def sample():
    n = int(sys.argv[2]) if len(sys.argv) > 2 and sys.argv[2].isdigit() else 600
    db = sqlite3.connect(f"file:{DB}?mode=ro", uri=True)
    ids = [r[0] for r in db.execute("SELECT id FROM canonical_jobs WHERE status = 'active' ORDER BY id")]
    picked = sorted(random.Random(1).sample(ids, min(n, len(ids))))
    save("sample.json", picked)
    print(f"{len(picked)} of {len(ids)} active jobs -> sample.json")


def jobs_text(ids):
    db = sqlite3.connect(f"file:{DB}?mode=ro", uri=True)
    out = {}
    for jid in ids:
        (job,) = db.execute("SELECT job FROM canonical_jobs WHERE id = ?", (jid,)).fetchone()
        j = json.loads(job)
        out[jid] = {
            "title": j["title"], "company": j["company"], "category": j.get("category"),
            "description": " ".join((j.get("descriptionMd") or "").split())[:1200],
        }
    return out


# ---------- label ----------

def vocabulary():
    v = with_home(BIN, lambda home: loker(BIN, home, "roles"))
    lines = []
    for r in v["roles"]:
        where = "family" if r["isFamily"] and not r.get("family") else f"in {r.get('family')}"
        if r.get("alsoIn"):
            where += f", also in {', '.join(r['alsoIn'])}"
        lines.append(f"- {r['id']} ({where}): {r['definition']}")
    mods = [f"- {m['id']}: {m['definition']}" for m in v["modifiers"]]
    ids = [r["id"] for r in v["roles"]]
    return "\n".join(lines), "\n".join(mods), ids, [m["id"] for m in v["modifiers"]], v["vocabularyVersion"]


PROMPT = """You classify job postings into a fixed role vocabulary.

Roles (families and the roles in them):
{roles}

Modifiers (how a job is done, any role):
{mods}

For each job:
- primary: the single most specific id that fits what the person in this job actually does. Use a family id only when the posting makes the family clear but not the role. Use "none" when nothing in the vocabulary fits (e.g. a line cook).
- secondary: up to 2 more ids only when the job genuinely is both (e.g. "Backend / Infrastructure Engineer"); usually empty. Never a family that already contains the primary.
- modifiers: forward_deployed and/or founding when they apply; usually empty.
- missing: when no id fits well, a short name for the role the vocabulary lacks; else "".
Judge the job itself (title, team, description), not keywords. "software" means engineering whose specialty the posting doesn't make clear.

Jobs:
{jobs}"""


def schema(ids, mods):
    return {"type": "object", "required": ["labels"], "properties": {"labels": {"type": "array", "items": {
        "type": "object", "required": ["i", "primary", "secondary", "modifiers", "missing"],
        "properties": {
            "i": {"type": "integer"},
            "primary": {"type": "string", "enum": ids + ["none"]},
            "secondary": {"type": "array", "items": {"type": "string", "enum": ids}, "maxItems": 2},
            "modifiers": {"type": "array", "items": {"type": "string", "enum": mods}},
            "missing": {"type": "string"},
        }}}}}


def ask_opus(prompt, sch, n):
    cmd = ["claude", "-p", "--model", JUDGES["opus"], "--tools", "", "--strict-mcp-config",
           "--no-session-persistence", "--setting-sources", "", "--output-format", "json",
           "--system-prompt", "You classify job postings. Reply only in the requested JSON.",
           "--json-schema", json.dumps(sch)]
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


def ask_deepseek(prompt, sch, n):
    body = json.dumps({
        "model": JUDGES["deepseek"], "temperature": 0, "response_format": {"type": "json_object"},
        "messages": [{"role": "user", "content": prompt + "\n\nReply with JSON only, shaped as: "
                      + json.dumps({"labels": [{"i": 0, "primary": "backend", "secondary": [], "modifiers": [], "missing": ""}]})}],
    }).encode()
    req = urllib.request.Request("https://api.deepseek.com/chat/completions", data=body, headers={
        "Content-Type": "application/json", "Authorization": "Bearer " + os.environ["DEEPSEEK_API_KEY"]})
    allowed = set(sch["properties"]["labels"]["items"]["properties"]["primary"]["enum"])
    err = None
    for _ in range(3):
        try:
            with urllib.request.urlopen(req, timeout=300) as r:
                out = json.loads(json.load(r)["choices"][0]["message"]["content"])
            labels = {l["i"]: l for l in out["labels"]}
            assert set(labels) == set(range(n)), sorted(labels)
            for l in labels.values():
                assert l["primary"] in allowed, l["primary"]
                l["secondary"] = [s for s in l.get("secondary", []) if s in allowed and s != "none"][:2]
                l.setdefault("modifiers", []); l.setdefault("missing", "")
            return [labels[i] for i in range(n)]
        except Exception as e:  # noqa: BLE001
            err = e
    raise RuntimeError(f"deepseek failed: {err}")


def label():
    judge = os.environ.get("JUDGE", "opus")
    roles, mods, ids, mod_ids, version = vocabulary()
    labels = load(f"labels-{judge}.json", {})
    key = lambda jid: f"{PROMPT_VERSION}|{version}|{jid}"
    todo = [j for j in load("sample.json", []) if key(j) not in labels]
    print(f"judge={judge}: {len(todo)} to label ({len(labels)} cached)", flush=True)
    texts = jobs_text(todo)
    sch = schema(ids, mod_ids)
    batches = [todo[i:i + 15] for i in range(0, len(todo), 15)]

    def one(batch):
        jobs = "\n---\n".join(
            f"[{n}] Title: {texts[j]['title']}\nCompany: {texts[j]['company']}\n"
            f"Source category: {texts[j]['category'] or '(none)'}\nDescription (start): {texts[j]['description']}"
            for n, j in enumerate(batch))
        prompt = PROMPT.format(roles=roles, mods=mods, jobs=jobs)
        ask = ask_opus if judge == "opus" else ask_deepseek
        return list(zip(batch, ask(prompt, sch, len(batch))))

    done = 0
    with ThreadPoolExecutor(4 if judge == "opus" else 8) as ex:
        for pairs in ex.map(one, batches):
            for jid, l in pairs:
                labels[key(jid)] = {k: l[k] for k in ("primary", "secondary", "modifiers", "missing")}
            done += 1
            if done % 5 == 0:
                save(f"labels-{judge}.json", labels)
                print(f"  {done}/{len(batches)} batches", flush=True)
    save(f"labels-{judge}.json", labels)
    print("done")


# ---------- Jev as a contender ----------

JEV_MODEL = "typesafe/jev-1.14-20261014"  # pinned: aliases move between releases


def jev():
    """Jev (TypeSafe, via OpenRouter Decisions) picks one role per sample job.
    Predictions, not labels: it is scored against the Opus labels."""
    v = with_home(BIN, lambda home: loker(BIN, home, "roles"))
    criteria = {r["id"]: f"{r['label']}: {r['definition']}" for r in v["roles"]}
    criteria["none"] = "None of these: the job is outside this vocabulary (e.g. cooking, driving, retail)."
    question = {"role": {"type": "choice",
                         "instructions": "What does the person in this job do? Pick the most specific option; "
                                         "a family only when the posting doesn't make the role clear.",
                         "criteria": criteria}}
    preds = load("predictions-jev.json", {})
    key = lambda jid: f"{JEV_MODEL}|{v['vocabularyVersion']}|{jid}"
    todo = [j for j in load("sample.json", []) if key(j) not in preds]
    texts = jobs_text(todo)
    print(f"jev: {len(todo)} to ask ({len(preds)} cached)", flush=True)

    def one(jid):
        t = texts[jid]
        state = {"title": t["title"], "company": t["company"], "category": t["category"] or "",
                 "description": t["description"][:800]}
        body = json.dumps({"model": JEV_MODEL, "state": state, "questions": question}).encode()
        req = urllib.request.Request("https://openrouter.ai/api/alpha/decisions", data=body, headers={
            "Authorization": "Bearer " + os.environ["OPENROUTER_API_KEY"], "Content-Type": "application/json"})
        err = None
        for _ in range(4):
            try:
                with urllib.request.urlopen(req, timeout=120) as r:
                    out = json.load(r)
                a = out["answers"]["role"]
                top = sorted(a["probabilities"].items(), key=lambda x: -x[1])[:3]
                return jid, {"choice": a["choice"], "confidence": a.get("confidence"), "top": top,
                             "cost": (out.get("usage") or {}).get("cost", 0)}
            except Exception as e:  # noqa: BLE001
                err = e
        raise RuntimeError(f"jev failed for {jid}: {err}")

    cost = 0.0
    with ThreadPoolExecutor(8) as ex:
        for i, (jid, p) in enumerate(ex.map(one, todo), 1):
            preds[key(jid)] = p
            cost += p["cost"] or 0
            if i % 50 == 0:
                save("predictions-jev.json", preds)
                print(f"  {i}/{len(todo)}", flush=True)
    save("predictions-jev.json", preds)
    print(f"done; ${cost:.4f}")


def jev_roles(version, ids):
    preds = load("predictions-jev.json", {})
    out = {}
    for jid in ids:
        p = preds.get(f"{JEV_MODEL}|{version}|{jid}")
        if p:
            out[jid] = None if p["choice"] == "none" else {
                "primary": p["choice"], "secondary": [], "modifiers": [], "method": "jev",
                "confidence": p["top"][0][1] if p["top"] else 0}
    return out


# ---------- score ----------

def vocab_tree(binary):
    v = with_home(binary, lambda home: loker(binary, home, "roles"))
    parents = {r["id"]: ([r["family"]] if r.get("family") else []) + r.get("alsoIn", []) for r in v["roles"]}
    return parents, v["vocabularyVersion"]


def within(parents, a, b):
    return a == b or any(within(parents, p, b) for p in parents.get(a, []))


def top(parents, r):
    while parents.get(r):
        r = parents[r][0]
    return r


def classified(binary, ids):
    def run(home):
        out = {}
        for i in range(0, len(ids), 100):
            for j in loker(binary, home, "get", *ids[i:i + 100])["jobs"]:
                out[j["id"]] = j
        return out
    return with_home(binary, run)


def score():
    binary = os.path.abspath(arg("--bin", BIN))
    judge = os.environ.get("JUDGE", "opus")
    parents, version = vocab_tree(binary)
    labels = load(f"labels-{judge}.json", {})
    ids = load("sample.json", [])
    gold = {j: labels.get(f"{PROMPT_VERSION}|{version}|{j}") for j in ids}
    gold = {j: g for j, g in gold.items() if g}
    got = classified(binary, list(gold))
    pred = arg("--pred", "rules")
    if pred != "rules":
        jv = jev_roles(version, list(gold))
        gold = {j: g for j, g in gold.items() if j in jv}
        threshold = float(pred.split(":")[1]) if ":" in pred else 0.0
        for jid in gold:
            rules = got[jid].get("role")
            if pred.startswith("jev"):
                got[jid]["role"] = jv[jid]
            # hybrid: the rules where a title names a role; Jev for the rest
            # when it is at least `threshold` sure, else the rules.
            elif not (rules and rules["method"] == "title" and rules["confidence"] >= 0.9):
                j = jv[jid]
                if j is None or j["confidence"] >= threshold:
                    got[jid]["role"] = j
    n = len(gold)
    c = Counter()
    by_method = defaultdict(Counter)
    confusion = Counter()
    examples = defaultdict(list)
    mods = defaultdict(Counter)
    for jid, g in gold.items():
        job = got[jid]
        role = job.get("role")
        mine = role["primary"] if role else "none"
        mine_all = [mine] + (role.get("secondary", []) if role else [])
        want = g["primary"]
        method = role["method"] if role else "none"
        exact = mine == want
        compatible = exact or (mine != "none" and want != "none" and (
            within(parents, mine, want) or within(parents, want, mine) or want in mine_all))
        same_family = mine != "none" and want != "none" and top(parents, mine) == top(parents, want)
        c["exact"] += exact
        c["compatible"] += compatible
        c["family"] += same_family or (mine == want == "none")
        c["covered"] += role is not None
        c["gold_none"] += want == "none"
        c["missed"] += role is None and want != "none"
        c["false_role"] += role is not None and want == "none"
        by_method[method]["n"] += 1
        by_method[method]["exact"] += exact
        by_method[method]["compatible"] += compatible
        if not compatible:
            confusion[(want, mine)] += 1
            if len(examples[(want, mine)]) < 3:
                examples[(want, mine)].append(job["title"])
        have = set(role.get("modifiers", [])) if role else set()
        for m in set(g["modifiers"]) | have:
            mods[m]["tp"] += m in have and m in g["modifiers"]
            mods[m]["fp"] += m in have and m not in g["modifiers"]
            mods[m]["fn"] += m not in have and m in g["modifiers"]
    pct = lambda x, d=n: f"{x / d:.0%}" if d else "-"
    n = len(gold)
    print(f"{n} labelled jobs (judge={judge}, vocabulary v{version}, predictor {pred}, {os.path.basename(binary)})")
    print(f"  covered (has a role)   {pct(c['covered'])}   gold says none: {c['gold_none']}")
    print(f"  exact primary          {pct(c['exact'])}")
    print(f"  compatible             {pct(c['compatible'])}   (same id, one inside the other, or gold primary among our roles)")
    print(f"  same top family        {pct(c['family'])}")
    print(f"  missed (no role, gold has one) {c['missed']}   false role (gold none) {c['false_role']}")
    print("  by method:", "  ".join(f"{m} {v['n']} (exact {pct(v['exact'], v['n'])}, compatible {pct(v['compatible'], v['n'])})"
                                   for m, v in sorted(by_method.items(), key=lambda x: -x[1]["n"])))
    for m, v in mods.items():
        p = v["tp"] / (v["tp"] + v["fp"]) if v["tp"] + v["fp"] else 1
        r = v["tp"] / (v["tp"] + v["fn"]) if v["tp"] + v["fn"] else 1
        print(f"  modifier {m}: precision {p:.0%} recall {r:.0%} ({v['tp']} agreed)")
    print("\n  incompatible (gold -> ours), most common:")
    for (w, m), k in confusion.most_common(25 if "-v" in sys.argv else 12):
        print(f"    {k:3}  {w:24} -> {m:24} e.g. {examples[(w, m)]}")
    missing = Counter(g["missing"].strip().lower() for g in gold.values() if g.get("missing", "").strip())
    if missing:
        print("\n  roles the judge found missing:", ", ".join(f"{k} ({v})" for k, v in missing.most_common(15)))


def agree():
    _, version = vocab_tree(BIN)
    a, b = load("labels-opus.json", {}), load("labels-deepseek.json", {})
    parents, _ = vocab_tree(BIN)
    keys = [k for k in a if k in b and k.split("|")[1] == str(version)]
    exact = sum(a[k]["primary"] == b[k]["primary"] for k in keys)
    fam = sum(top(parents, a[k]["primary"]) == top(parents, b[k]["primary"]) for k in keys)
    print(f"{len(keys)} jobs labelled by both: same primary {exact / len(keys):.0%}, same top family {fam / len(keys):.0%}")
    diff = Counter((a[k]["primary"], b[k]["primary"]) for k in keys if a[k]["primary"] != b[k]["primary"])
    print("most common disagreements (opus, deepseek):", diff.most_common(10))


if __name__ == "__main__":
    cmds = {"sample": sample, "label": label, "score": score, "agree": agree, "jev": jev}
    if len(sys.argv) < 2 or sys.argv[1] not in cmds:
        sys.exit(__doc__)
    cmds[sys.argv[1]]()
