"""Search-ranking eval for loker. Run by hand; not part of CI. See README.md.

  python3 eval.py snapshot [path]   # freeze a copy of $LOKER_HOME/loker.db -> snapshot.db
  python3 eval.py pool              # run every variant, write pool.json (top-K per variant)
  JUDGE=opus|deepseek python3 eval.py judge   # grade unjudged (query, job) pairs, cached
  JUDGE=opus python3 eval.py score [-q] [--holdout]   # metrics per variant
  python3 eval.py agree             # agreement between the two judges
  python3 eval.py sample [n]        # blind human spot-check sheet

Variants are SQL over FTS5 tables the eval builds itself from the snapshot,
in the same shape as store.rs (quoted terms; AND = space, anyWord = OR;
ORDER BY bm25, id). Topical relevance only: eligibility is out of scope.
"""
import hashlib, json, math, os, random, sqlite3, subprocess, sys, tempfile, threading, urllib.request
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
DB = os.environ.get("LOKER_EVAL_DB", os.path.join(HERE, "snapshot.db"))
K = 20
PROMPT_VERSION = "v1"
DESC_CHARS = 2500


def quote(t):
    return '"%s"' % t.replace('"', '""')


def fts(q, any_word):
    terms = [quote(t) for t in q.split()]
    return (" OR " if any_word else " ").join(terms)


SYN = {
    "k8s": ["kubernetes"], "kubernetes": ["k8s"],
    "golang": ["go"], "sre": ["site reliability"],
    "ml": ["machine learning"], "llm": ["llms", "large language model", "genai"],
    "ai": ["artificial intelligence", "genai"],
    "js": ["javascript"], "ts": ["typescript"],
    "frontend": ["front end"], "backend": ["back end"],
    "fullstack": ["full stack"], "postgres": ["postgresql"], "postgresql": ["postgres"],
    "qa": ["quality assurance"], "nextjs": ["next.js"],
}


def fts_syn(q, any_word):
    groups = []
    for t in q.lower().split():
        alts = [t] + SYN.get(t, [])
        groups.append(quote(t) if len(alts) == 1 else "(" + " OR ".join(quote(a) for a in alts) + ")")
    return (" OR " if any_word else " AND ").join(groups)


def bm25(table, w):
    return f"bm25({table})" if w is None else f"bm25({table}, 1, {w[0]}, {w[1]}, {w[2]})"


def run(db, table, q, any_word, w, k=K, build=fts):
    sql = (f"SELECT f.job_id FROM {table} f JOIN canonical_jobs c ON c.id = f.job_id "
           f"WHERE {table} MATCH ? AND c.status = 'active' ORDER BY {bm25(table, w)}, c.id LIMIT ?")
    return [r[0] for r in db.execute(sql, (build(q, any_word), k))]


def and_then_or(db, table, q, w, k=K, build=fts, fill_below=None):
    first = run(db, table, q, False, w, k, build)
    if fill_below is not None:
        total = len(run(db, table, q, False, w, 10_000, build))
        if total >= fill_below:
            return first
    rest = [j for j in run(db, table, q, True, w, k, build) if j not in first]
    return (first + rest)[:k]


T5 = (5, 1, 1)
T10 = (10, 2, 1)
PLAIN, PORTER = "eval_plain", "eval_porter"
VARIANTS = {
    "plain and": lambda db, q: run(db, PLAIN, q, False, None),
    "plain or": lambda db, q: run(db, PLAIN, q, True, None),
    "plain and t5": lambda db, q: run(db, PLAIN, q, False, T5),
    "plain or t5": lambda db, q: run(db, PLAIN, q, True, T5),
    "plain or t10": lambda db, q: run(db, PLAIN, q, True, T10),
    "plain and->or t5": lambda db, q: and_then_or(db, PLAIN, q, T5),
    "porter or": lambda db, q: run(db, PORTER, q, True, None),
    "porter or t5": lambda db, q: run(db, PORTER, q, True, T5),
    "porter and->or t5": lambda db, q: and_then_or(db, PORTER, q, T5),
    "porter and->or<1 t5": lambda db, q: and_then_or(db, PORTER, q, T5, fill_below=1),
    "porter and->or<5 t5": lambda db, q: and_then_or(db, PORTER, q, T5, fill_below=5),
    "porter and->or<10 t5": lambda db, q: and_then_or(db, PORTER, q, T5, fill_below=10),
    "syn plain and->or t5": lambda db, q: and_then_or(db, PLAIN, q, T5, build=fts_syn),
    "syn porter and->or t5": lambda db, q: and_then_or(db, PORTER, q, T5, build=fts_syn),
}
BASELINE = "plain and"


def connect():
    if not os.path.exists(DB):
        sys.exit(f"no snapshot at {DB}; run `python3 eval.py snapshot` first")
    db = sqlite3.connect(DB)
    for table, tok in ((PLAIN, "unicode61 remove_diacritics 2"), (PORTER, "porter unicode61 remove_diacritics 2")):
        if not db.execute("SELECT 1 FROM sqlite_master WHERE name = ?", (table,)).fetchone():
            db.executescript(f"""
              CREATE VIRTUAL TABLE {table} USING fts5(
                job_id UNINDEXED, title, company, description, tokenize = '{tok}');
              INSERT INTO {table} SELECT job_id, title, company, description FROM jobs_fts;
            """)
            db.commit()
    return db


def snapshot():
    src = sys.argv[2] if len(sys.argv) > 2 else os.path.join(
        os.environ.get("LOKER_HOME", os.path.expanduser("~/.loker")), "loker.db")
    if os.path.exists(DB):
        sys.exit(f"{DB} exists; move it away first (labels are tied to its job ids)")
    with sqlite3.connect(src) as s_, sqlite3.connect(DB) as d:
        s_.backup(d)
    n = sqlite3.connect(DB).execute("SELECT count(*) FROM canonical_jobs WHERE status = 'active'").fetchone()[0]
    print(f"{src} -> {DB}: {n} active jobs")


def load(name, default):
    p = os.path.join(HERE, name)
    return json.load(open(p)) if os.path.exists(p) else default


def save(name, obj):
    with open(os.path.join(HERE, name), "w") as f:
        json.dump(obj, f, indent=1, sort_keys=True)


def queries():
    return load("queries.json", [])


def pool():
    db = connect()
    out = {}
    for v, fn in VARIANTS.items():
        out[v] = {qq["q"]: fn(db, qq["q"]) for qq in queries()}
    save("pool.json", out)
    pairs = {(q, j) for runs in out.values() for q, ids in runs.items() for j in ids}
    print(f"{len(VARIANTS)} variants, {len(queries())} queries, {len(pairs)} unique (query, job) pairs")


RUBRIC = """Grade how well each job matches what the user is looking for:
3 = exactly the kind of role sought
2 = clearly relevant, though not a perfect fit (adjacent specialty, or the sought skill is secondary)
1 = tangential (mentions the topic, but the role itself is something else)
0 = not relevant

Judge the role itself, not whether the query words appear in the text. Ignore location,
eligibility, pay and seniority unless the intent asks about them."""

JUDGE = """You grade search results for a job search engine.

Query typed by the user: "{q}"
What the user is looking for: {intent}

Job posting:
Title: {title}
Company: {company}
Description (may be truncated):
{desc}

""" + RUBRIC + """
Reply with JSON only: {{"reason": "<one short sentence>", "grade": <0-3>}}"""

BATCH = """You grade search results for a job search engine. Grade every job independently;
don't grade on a curve within the batch.

Query typed by the user: "{q}"
What the user is looking for: {intent}

""" + RUBRIC + """

Jobs:
{jobs}

Return a grade for every job index."""

PROMPTS = {"v1": "e934178442fa"}


def prompt_hash():
    return hashlib.sha256(f"{JUDGE}|{BATCH}|{DESC_CHARS}".encode()).hexdigest()[:12]


JUDGES = {"deepseek": ("labels-deepseek.json", "deepseek-v4-pro"), "opus": ("labels-opus.json", "claude-opus-5-5")}


def which():
    return os.environ.get("JUDGE", "opus")


def key(qq, job_id, judge=None):
    model = JUDGES[judge or which()][1]
    h = hashlib.sha256(f"{PROMPT_VERSION}|{model}|{qq['q']}|{qq['intent']}".encode()).hexdigest()[:12]
    return f"{h}|{job_id}"


def labels_for(judge=None):
    return load(JUDGES[judge or which()][0], {})


def ask_deepseek(prompt):
    body = json.dumps({
        "model": JUDGES["deepseek"][1], "temperature": 0,
        "response_format": {"type": "json_object"},
        "messages": [{"role": "user", "content": prompt}],
    }).encode()
    req = urllib.request.Request("https://api.deepseek.com/chat/completions", data=body, headers={
        "Content-Type": "application/json",
        "Authorization": "Bearer " + os.environ["DEEPSEEK_API_KEY"],
    })
    err = None
    for _ in range(3):
        try:
            with urllib.request.urlopen(req, timeout=120) as r:
                out = json.loads(json.load(r)["choices"][0]["message"]["content"])
            g = int(out["grade"])
            assert 0 <= g <= 3
            return {"grade": g, "reason": out.get("reason", "")}
        except Exception as e:  # noqa: BLE001
            err = e
    raise RuntimeError(f"judge failed: {err}")


SCHEMA = json.dumps({"type": "object", "required": ["grades"], "properties": {"grades": {
    "type": "array", "items": {"type": "object", "required": ["i", "grade", "reason"], "properties": {
        "i": {"type": "integer"}, "grade": {"type": "integer", "minimum": 0, "maximum": 3},
        "reason": {"type": "string"}}}}}})


def ask_opus(prompt, n):
    cmd = ["claude", "-p", "--model", JUDGES["opus"][1], "--tools", "", "--strict-mcp-config",
           "--no-session-persistence", "--setting-sources", "", "--output-format", "json",
           "--system-prompt", "You grade search results for a job search engine. Reply only in the requested JSON.",
           "--json-schema", SCHEMA]
    err = None
    with tempfile.TemporaryDirectory(prefix="loker-judge-") as cwd:
        for _ in range(3):
            try:
                p = subprocess.run(cmd, input=prompt, capture_output=True, text=True, cwd=cwd, timeout=600)
                out = json.loads(p.stdout)
                grades = {g["i"]: g for g in out["structured_output"]["grades"]}
                assert set(grades) == set(range(n)), f"got indices {sorted(grades)}"
                return ([{"grade": grades[i]["grade"], "reason": grades[i]["reason"]} for i in range(n)],
                        out.get("total_cost_usd", 0))
            except Exception as e:  # noqa: BLE001
                err = e
    raise RuntimeError(f"opus judge failed: {err}")


def judge():
    if PROMPTS.get(PROMPT_VERSION) != prompt_hash():
        sys.exit(f"the judge prompt changed: bump PROMPT_VERSION and add {PROMPT_VERSION!r}'s successor "
                 f"to PROMPTS as {prompt_hash()!r}")
    j = which()
    labels = labels_for(j)
    pooled = load("pool.json", {})
    qs = {qq["q"]: qq for qq in queries()}
    todo = sorted({(q, jid) for runs in pooled.values() for q, ids in runs.items() for jid in ids
                   if q in qs and key(qs[q], jid, j) not in labels})
    print(f"judge={j}: {len(todo)} pairs to judge ({len(labels)} cached)", flush=True)
    local = threading.local()

    def row(jid):
        if not hasattr(local, "db"):
            local.db = sqlite3.connect(DB)
        return local.db.execute("SELECT title, company, description FROM jobs_fts WHERE job_id = ?", (jid,)).fetchone()

    if j == "deepseek":
        def one(pair):
            q, jid = pair
            t, c, d = row(jid)
            return [(key(qs[q], jid, j), ask_deepseek(JUDGE.format(q=q, intent=qs[q]["intent"], title=t, company=c, desc=d[:DESC_CHARS])))]
        tasks, workers = todo, 12
    else:
        by_q = {}
        for q, jid in todo:
            by_q.setdefault(q, []).append(jid)
        tasks = [(q, ids[i:i + 10]) for q, ids in by_q.items() for i in range(0, len(ids), 10)]

        def one(task):
            q, ids = task
            jobs = []
            for i, jid in enumerate(ids):
                t, c, d = row(jid)
                jobs.append(f"[{i}] Title: {t}\nCompany: {c}\nDescription (truncated):\n{d[:DESC_CHARS]}\n")
            res, cost = ask_opus(BATCH.format(q=q, intent=qs[q]["intent"], jobs="\n---\n".join(jobs)), len(ids))
            one.cost += cost
            return [(key(qs[q], jid, j), r) for jid, r in zip(ids, res)]
        one.cost = 0.0
        workers = 4

    done = 0
    with ThreadPoolExecutor(workers) as ex:
        for pairs in ex.map(one, tasks):
            for k_, v in pairs:
                labels[k_] = v
            done += 1
            if done % 10 == 0:
                save(JUDGES[j][0], labels)
                print(f"  {done}/{len(tasks)} calls", flush=True)
    save(JUDGES[j][0], labels)
    if j == "opus":
        print(f"notional cost ${one.cost:.2f} (subscription)")
    print("done")


def ndcg(ranked, grades, k=10):
    dcg = sum((2 ** grades.get(j, 0) - 1) / math.log2(i + 2) for i, j in enumerate(ranked[:k]))
    ideal = sorted(grades.values(), reverse=True)[:k]
    idcg = sum((2 ** g - 1) / math.log2(i + 2) for i, g in enumerate(ideal))
    return dcg / idcg if idcg else 0.0


def score():
    j = which()
    labels = labels_for(j)
    pooled = load("pool.json", {})
    allq = queries()
    mean = lambda xs: sum(xs) / len(xs) if xs else float("nan")

    def metrics(qs):
        per = {}
        for v, runs in pooled.items():
            per[v] = []
            for qq in qs:
                grades = {}
                for runs2 in pooled.values():
                    for jid in runs2[qq["q"]]:
                        lab = labels.get(key(qq, jid, j))
                        if lab:
                            grades[jid] = lab["grade"]
                rel = {x for x, g in grades.items() if g >= 2}
                ranked = runs[qq["q"]]
                unj = sum(1 for x in ranked[:K] if key(qq, x, j) not in labels)
                per[v].append({"ndcg10": ndcg(ranked, grades),
                               "p10": sum(1 for x in ranked[:10] if x in rel) / 10,
                               "r20": (len(rel & set(ranked[:20])) / len(rel)) if rel else 1.0,
                               "empty": 1 if not ranked else 0, "n": len(ranked), "rel": len(rel), "unj": unj,
                               "junk": sum(1 for x in ranked[:K] if x not in rel)})
        return per

    base_flag = sys.argv[sys.argv.index("--base") + 1] if "--base" in sys.argv else BASELINE

    def table(title, qs, base_name=base_flag):
        per = metrics(qs)
        base = per[base_name]

        def boot(v, m, iters=5000):
            d = [a[m] - b[m] for a, b in zip(per[v], base)]
            rnd = random.Random(0)
            ms = sorted(mean([rnd.choice(d) for _ in d]) for _ in range(iters))
            return mean(d), ms[int(0.025 * iters)], ms[int(0.975 * iters)]

        print(f"\n## {title} ({len(qs)} queries; judge={j}; relevant = grade>=2; K={K})")
        print(f"{'variant':24} {'nDCG@10':>8} {'P@10':>6} {'R@20':>6} {'empty':>6} {'unjudged':>9} {'results':>8} {'junk':>5}   ΔnDCG vs {base_name} [95% CI]")
        for v in pooled:
            r = per[v]
            d, lo, hi = boot(v, "ndcg10")
            delta = "" if v == base_name else f"{d:+.3f} [{lo:+.3f}, {hi:+.3f}]"
            print(f"{v:24} {mean([x['ndcg10'] for x in r]):8.3f} {mean([x['p10'] for x in r]):6.2f} "
                  f"{mean([x['r20'] for x in r]):6.2f} {sum(x['empty'] for x in r):6d} {sum(x['unj'] for x in r):9d} "
                  f"{mean([x['n'] for x in r]):8.1f} {mean([x['junk'] for x in r]):5.1f}   {delta}")
        return per

    tune = [q for q in allq if not q.get("holdout") and not q.get("expect_none")]
    hold = [q for q in allq if q.get("holdout") and not q.get("expect_none")]
    none = [q for q in allq if q.get("expect_none")]
    per = table("tuning set", tune)
    if "--holdout" in sys.argv:
        table("HOLDOUT", hold)
    print(f"\n## should-match-nothing queries: results returned / judged relevant")
    for v, runs in pooled.items():
        cells = []
        for qq in none:
            ids = runs[qq["q"]]
            rel = sum(1 for x in ids if labels.get(key(qq, x, j), {}).get("grade", 0) >= 2)
            cells.append(f"{qq['q'][:14]}={len(ids)}/{rel}")
        print(f"{v:24} " + "  ".join(cells))
    if "-q" in sys.argv:
        names = list(pooled)
        print("\nper query nDCG@10 (relevant in pool):")
        print(f"{'query':38}" + "".join(f"{n[:11]:>12}" for n in names))
        for i, qq in enumerate(tune):
            print(f"{qq['q'][:31]:31} ({per[names[0]][i]['rel']:3})" + "".join(f"{per[n][i]['ndcg10']:12.2f}" for n in names))


def agree():
    a, b = labels_for("deepseek"), labels_for("opus")
    pooled = load("pool.json", {})
    pairs = []
    for qq in queries():
        for jid in {x for runs in pooled.values() for x in runs[qq["q"]]}:
            la, lb = a.get(key(qq, jid, "deepseek")), b.get(key(qq, jid, "opus"))
            if la and lb:
                pairs.append((la["grade"], lb["grade"]))
    n = len(pairs)
    exact = sum(x == y for x, y in pairs) / n
    bx = [(x >= 2, y >= 2) for x, y in pairs]
    po = sum(x == y for x, y in bx) / n
    pa, pb = sum(x for x, _ in bx) / n, sum(y for _, y in bx) / n
    pe = pa * pb + (1 - pa) * (1 - pb)
    print(f"{n} pairs judged by both: exact grade {exact:.0%}, binary (>=2) {po:.0%}, kappa {(po - pe) / (1 - pe):.2f}")
    print(f"relevant rate: deepseek {pa:.0%}, opus {pb:.0%}; mean grade diff (opus-deepseek) {sum(y - x for x, y in pairs) / n:+.2f}")


def sample():
    labels = labels_for("opus")
    pooled = load("pool.json", {})
    rnd = random.Random(1)
    db = sqlite3.connect(DB)
    by_grade = {g: [] for g in range(4)}
    for qq in queries():
        for jid in sorted({x for runs in pooled.values() for x in runs[qq["q"]]}):
            lab = labels.get(key(qq, jid, "opus"))
            if lab:
                by_grade[lab["grade"]].append((qq, jid, lab["grade"]))
    n = int(sys.argv[2]) if len(sys.argv) > 2 else 60
    picks = []
    for g, items in by_grade.items():
        picks += rnd.sample(items, min(len(items), n // 4))
    rnd.shuffle(picks)
    lines, keyf = ["# Spot-check: grade each 0-3 (3 exact fit, 2 relevant, 1 tangential, 0 not)\n"], []
    for i, (qq, jid, g) in enumerate(picks, 1):
        t, c, d = db.execute("SELECT title, company, substr(description,1,400) FROM jobs_fts WHERE job_id=?", (jid,)).fetchone()
        lines.append(f"## {i}. query: `{qq['q']}`  ({qq['intent']})\n**{t}** at {c}  `{jid}`\n> {' '.join(d.split())[:300]}\n\nyour grade: \n")
        keyf.append({"i": i, "q": qq["q"], "job": jid, "opus": g})
    open(os.path.join(HERE, "spotcheck.md"), "w").write("\n".join(lines))
    save("spotcheck-key.json", keyf)
    print(f"{len(picks)} items -> spotcheck.md")


if __name__ == "__main__":
    cmds = {"snapshot": snapshot, "pool": pool, "judge": judge, "score": score, "agree": agree, "sample": sample}
    if len(sys.argv) < 2 or sys.argv[1] not in cmds:
        sys.exit(__doc__)
    cmds[sys.argv[1]]()
