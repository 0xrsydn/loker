"""Visa sponsorship eval for loker: `visa_text::visa_from_text` against
sponsorship labelled by Opus and DeepSeek. Run by hand; not part of CI.
See README.md.

  python3 visa_eval.py sample                    # draw sample.json (tuning) and holdout.json, once
  python3 visa_eval.py sample2                   # draw holdout2.json (new companies), once
  python3 visa_eval.py sample3                   # draw holdout3.json (new boards), once
  python3 visa_eval.py sample4                   # draw holdout4.json (more new boards), once
  JUDGE=opus|deepseek python3 visa_eval.py label [--set holdout]
  python3 visa_eval.py agree [--set holdout]     # Opus vs DeepSeek
  python3 visa_eval.py score [--set holdout] [--bin PATH] [-v]

The tuning sample is from the pay eval's snapshot (2026-11-19); the holdout
is from the role eval's fresh jobs (new boards, fetched later), without the
golden set's jobs. Never print a holdout job's text or labels while writing
the rules; add any job you read to seen.json.
"""
import json, os, random, re, sqlite3, subprocess, sys, tempfile, urllib.request
from collections import Counter
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
EVAL = os.path.dirname(HERE)
REPO = os.path.dirname(EVAL)
DBS = {"sample": os.path.join(EVAL, "pay", "snapshot.db"), "holdout": os.path.join(EVAL, "role", "snapshot-fresh.db"),
       "holdout2": os.path.join(EVAL, "role", "snapshot-fresh.db"), "holdout3": os.path.join(HERE, "snapshot-boards.db"),
       "holdout4": os.path.join(HERE, "snapshot-boards2.db")}
BIN = os.path.join(REPO, "target", "release", "examples", "visa_text")
PROMPT_VERSION = "v1"
JUDGES = {"opus": "claude-opus-5-5", "deepseek": "deepseek-v4-pro"}
MAX_CHARS = 15000
ANSWERS = ["offered", "limited", "not_offered", None]

MENTION = re.compile(r"(?i)visa|sponsor|work(?:ing)?[ -](?:authori[sz]ation|permit)|right to work|"
                     r"authori[sz]ed to work|immigration|\bH-?1B\b|green card")


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
    return s.get("mention", []) + s.get("rest", [])


def texts(name=SET):
    return descriptions(DBS[name], set(set_ids(name)))


def sample():
    if os.path.exists(os.path.join(HERE, "sample.json")):
        sys.exit("sets already drawn")
    golden = {i["id"] for i in json.load(open(os.path.join(EVAL, "golden", "items.json")))["items"]}
    fresh = set(json.load(open(os.path.join(EVAL, "role", "fresh.json")))["ids"])
    pools = {"sample": (descriptions(DBS["sample"]), 200),
             "holdout": ({j: d for j, d in descriptions(DBS["holdout"], fresh).items() if j not in golden}, 150)}
    for name, (jobs, n) in pools.items():
        mention = sorted(j for j, d in jobs.items() if MENTION.search(d["description"]))
        rest = sorted(set(jobs) - set(mention))
        rng = random.Random(f"visa-{name}")
        a = rng.sample(mention, min(n * 2 // 3, len(mention)))
        b = rng.sample(rest, min(n - len(a), len(rest)))
        save(f"{name}.json", {"mention": sorted(a), "rest": sorted(b)})
        print(f"{name}: {len(a)} of {len(mention)} mentioning visa words, {len(b)} of {len(rest)} others")
    s, h = set(set_ids("sample")), set(set_ids("holdout"))
    assert not s & h


def sample2():
    if os.path.exists(os.path.join(HERE, "holdout2.json")):
        sys.exit("holdout2 already drawn")
    old = descriptions(DBS["sample"])
    old_companies = {d["company"].lower() for d in old.values()}
    used = set(set_ids("sample")) | set(set_ids("holdout")) | set(load("seen.json", []))
    used |= {i["id"] for i in json.load(open(os.path.join(EVAL, "golden", "items.json")))["items"]}
    used |= set(json.load(open(os.path.join(EVAL, "role", "fresh.json")))["ids"])
    jobs = {j: d for j, d in descriptions(DBS["holdout2"]).items()
            if j not in old and j not in used and d["company"].lower() not in old_companies}
    rng = random.Random("visa-holdout2")
    out = {}
    for stratum, n in (("mention", 100), ("rest", 50)):
        pool = sorted(j for j, d in jobs.items() if bool(MENTION.search(d["description"])) == (stratum == "mention"))
        rng.shuffle(pool)
        per, picked = Counter(), []
        for j in pool:
            c = jobs[j]["company"].lower()
            if per[c] < 3:
                per[c] += 1
                picked.append(j)
            if len(picked) == n:
                break
        out[stratum] = sorted(picked)
        print(f"holdout2 {stratum}: {len(picked)} of {len(pool)}, {len(per)} companies")
    save("holdout2.json", out)


def sample3(name="holdout3"):
    if os.path.exists(os.path.join(HERE, f"{name}.json")):
        sys.exit(f"{name} already drawn")
    jobs = descriptions(DBS[name])
    rng = random.Random(f"visa-{name}")
    out = {}
    for stratum, n, cap in (("mention", 120, 8), ("rest", 30, 2)):
        pool = sorted(j for j, d in jobs.items() if bool(MENTION.search(d["description"])) == (stratum == "mention"))
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


PROMPT = """You read whether a job posting offers visa sponsorship, exactly as the posting states it.

For each job, give:
- sponsorship, one of:
  - "offered": the employer will sponsor a visa or work permit for this role, or supports the hire's visa or immigration process ("visa sponsorship available", "we sponsor visas", "relocation and visa support").
  - "limited": sponsorship only in some cases: "may", "case by case", "for exceptional candidates", only visa transfers, only some locations or levels of this posting.
  - "not_offered": the employer won't sponsor for this role, or the hire must not need sponsorship now or in the future ("unable to sponsor", "must not require sponsorship").
  - null: the posting says nothing about sponsorship.
- evidence: the sentence you read it from, quoted, or "".

Rules:
- Only what the posting says about sponsorship. "Must be authorized to work in the US" alone is null: it doesn't say whether the employer sponsors.
- Citizenship or security clearance requirements alone are null. Equal opportunity statements ("regardless of citizenship or immigration status") are null.
- "Sponsor" in another sense (event sponsors, an executive sponsor) is not visa sponsorship.
- If the posting says both (sponsors in one country, not another), answer "limited".

Jobs:
{jobs}"""

SCHEMA = {"type": "object", "required": ["labels"], "properties": {"labels": {"type": "array", "items": {
    "type": "object", "required": ["i", "sponsorship", "evidence"], "properties": {
        "i": {"type": "integer"},
        "sponsorship": {"type": ["string", "null"], "enum": ANSWERS},
        "evidence": {"type": "string"}}}}}}


def ask_opus(prompt, n):
    cmd = ["claude", "-p", "--model", JUDGES["opus"], "--tools", "", "--strict-mcp-config",
           "--no-session-persistence", "--setting-sources", "", "--output-format", "json",
           "--system-prompt", "You read visa sponsorship from job postings. Reply only in the requested JSON.",
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
    shape = {"labels": [{"i": 0, "sponsorship": "not_offered", "evidence": "..."}]}
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
            assert all(l.get("sponsorship") in ANSWERS for l in labels.values())
            return [labels[i] for i in range(n)]
        except Exception as e:  # noqa: BLE001
            err = e
    raise RuntimeError(f"deepseek failed: {err}")


def key(jid):
    return f"{PROMPT_VERSION}|{jid}"


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
            f"Description: {' '.join(t[j]['description'].split())[:MAX_CHARS]}"
            for n, j in enumerate(batch))
        ask = ask_opus if judge == "opus" else ask_deepseek
        return list(zip(batch, ask(PROMPT.format(jobs=jobs), len(batch))))

    done = 0
    with ThreadPoolExecutor(4 if judge == "opus" else 8) as ex:
        for pairs in ex.map(one, batches):
            for jid, l in pairs:
                labels[key(jid)] = {"sponsorship": l["sponsorship"], "evidence": l.get("evidence", "")}
            done += 1
            if done % 5 == 0:
                save(f"labels-{judge}.json", labels)
                print(f"  {done}/{len(batches)} batches", flush=True)
    save(f"labels-{judge}.json", labels)
    print("done")


def agree():
    a, b = load("labels-opus.json", {}), load("labels-deepseek.json", {})
    ids = [j for j in set_ids() if key(j) in a and key(j) in b]
    same = sum(a[key(j)]["sponsorship"] == b[key(j)]["sponsorship"] for j in ids)
    print(f"{SET}: {len(ids)} labelled by both, same answer {same}/{len(ids)}")
    print("  ", Counter((a[key(j)]["sponsorship"], b[key(j)]["sponsorship"]) for j in ids
                       if a[key(j)]["sponsorship"] != b[key(j)]["sponsorship"]))


def predicted(binary, ids, t):
    lines = "".join(json.dumps({"id": j, "text": t[j]["description"]}) + "\n" for j in ids)
    out = subprocess.run([binary], input=lines, capture_output=True, text=True, check=True).stdout
    return {r["id"]: r["visa"] for r in (json.loads(l) for l in out.split("\n") if l)}


def score():
    judge = os.environ.get("JUDGE", "opus")
    labels = load(f"labels-{judge}.json", {})
    ids = [j for j in set_ids() if key(j) in labels]
    t = texts()
    got = predicted(arg("--bin", BIN), ids, t)
    gold = {j: labels[key(j)]["sponsorship"] for j in ids}
    ours = {j: (got.get(j) or {}).get("sponsorship") for j in ids}
    print(f"{len(ids)} labelled jobs in {SET}.json (judge={judge})")
    claims = [j for j in ids if ours[j]]
    stated = [j for j in ids if gold[j]]
    right = [j for j in claims if ours[j] == gold[j]]
    print(f"  claims  precision {len(right)}/{len(claims)}   recall {sum(ours[j] == gold[j] for j in stated)}/{len(stated)}")
    for a in ANSWERS[:3]:
        c = [j for j in ids if ours[j] == a]
        s = [j for j in ids if gold[j] == a]
        print(f"  {a:12} precision {sum(gold[j] == a for j in c)}/{len(c)}   recall {sum(ours[j] == a for j in s)}/{len(s)}")
    print("  confusion (label -> ours):", dict(Counter((gold[j], ours[j]) for j in ids if gold[j] != ours[j])))
    if "-v" in sys.argv and SET not in ("holdout", "holdout4"):
        for j in ids:
            if gold[j] != ours[j]:
                print(f"\n{j} label={gold[j]} ours={ours[j]}\n  label: {labels[key(j)]['evidence'][:300]}"
                      f"\n  ours:  {((got.get(j) or {}).get('evidence') or '')[:300]}")


if __name__ == "__main__":
    cmds = {"sample": sample, "sample2": sample2, "sample3": sample3, "sample4": lambda: sample3("holdout4"), "label": label, "agree": agree, "score": score}
    if len(sys.argv) < 2 or sys.argv[1] not in cmds:
        sys.exit(__doc__)
    cmds[sys.argv[1]]()
