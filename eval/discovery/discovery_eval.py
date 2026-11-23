import argparse, json, random, re, sqlite3, sys
from collections import Counter, defaultdict

SUFFIXES = {"inc", "llc", "ltd", "limited", "gmbh", "corp", "corporation", "co", "plc", "pte", "pty",
            "sa", "sas", "bv", "ag", "oy", "ab", "srl", "sl", "kk", "tbk", "pt"}
SUSPICIOUS = re.compile(r"\b(test|testing|demo|sandbox|staging|template|example|dummy|do not use|careers?|jobs?)\b", re.I)
ATS = ("ashby", "greenhouse", "lever")


def company_key(name):
    words = [re.sub(r"[^\w]", "", w).replace("_", "") for w in re.split(r"[\s,]+", (name or "").lower())]
    words = [w for w in words if w]
    while len(words) > 1 and words[-1] in SUFFIXES:
        words.pop()
    return "".join(words)


def pct(n, d):
    return f"{100 * n / d:.0f}%" if d else "-"


def boards(db):
    con = sqlite3.connect(db)
    con.row_factory = sqlite3.Row
    rows = [dict(r) for r in con.execute("SELECT * FROM boards")]
    finds = defaultdict(set)
    for key, finder in con.execute("SELECT key, finder FROM board_finds"):
        finds[key].add(finder.split(":")[0] if finder.startswith("commoncrawl") else finder)
    for r in rows:
        r["finders"] = finds[r["key"]]
    return rows


def show_funnel(rows):
    print("## Funnel (every candidate checked against its ATS API)\n")
    print(f"{'ats':<11}{'found':>7}{'live':>7}{'empty':>7}{'missing':>8}{'no answer':>10}{'live jobs':>11}{'jobs/board':>11}")
    for ats in ATS + ("all",):
        rs = [r for r in rows if ats in ("all", r["ats"])]
        st = Counter(r["status"] for r in rs)
        live = [r for r in rs if r["status"] == "live"]
        counted = [r["jobs"] for r in live if r["jobs"] is not None]
        jobs = sum(counted)
        per = f"{jobs / len(counted):.1f}" if counted else "-"
        print(f"{ats:<11}{len(rs):>7}{st['live']:>7}{st['empty']:>7}{st['missing']:>8}{st['unchecked']:>10}"
              f"{jobs if counted else '-':>11}{per:>11}")
    errors = Counter((r["error"] or "")[:90] for r in rows if r["status"] == "unchecked")
    if errors:
        print("\nno answer, most common errors:")
        for e, n in errors.most_common(5):
            print(f"  {n:>5}  {e}")
    print()


def show_finders(rows):
    print("## Where live boards came from\n")
    by = Counter()
    for r in rows:
        if r["status"] == "live":
            by[" + ".join(sorted(r["finders"]))] += 1
    for k, n in by.most_common():
        print(f"  {n:>5}  {k}")
    print()


def show_names(rows, fd_names):
    live = [r for r in rows if r["status"] == "live"]
    print("## Company names on live boards\n")
    print(f"{'ats':<11}{'live':>6}{'named':>7}{'website':>9}{'slug = name':>13}")
    for ats in ATS:
        rs = [r for r in live if r["ats"] == ats]
        named = [r for r in rs if r["name"]]
        same = [r for r in named if company_key(r["slug"]) == company_key(r["name"])]
        print(f"{ats:<11}{len(rs):>6}{pct(len(named), len(rs)):>7}{pct(sum(1 for r in rs if r['website']), len(rs)):>9}"
              f"{pct(len(same), len(named)):>13}")
    print("\n`slug = name`: the slug alone, case and punctuation aside, already is the name.")
    if fd_names:
        pairs = [(r, fd_names[(r["ats"], r["key"])]) for r in live if r["name"] and (r["ats"], r["key"]) in fd_names]
        exact = [p for p in pairs if company_key(p[0]["name"]) == company_key(p[1])]
        close = [p for p in pairs if p not in exact and
                 (company_key(p[1]) in company_key(p[0]["name"]) or company_key(p[0]["name"]) in company_key(p[1]))]
        other = [p for p in pairs if p not in exact and p not in close]
        print(f"\nAgainst forwarddeployed.fun's hand-kept names ({len(pairs)} boards on both lists):")
        print(f"  same name           {len(exact):>4}  {pct(len(exact), len(pairs))}")
        print(f"  one contains other  {len(close):>4}  {pct(len(close), len(pairs))}")
        print(f"  different           {len(other):>4}  {pct(len(other), len(pairs))}")
        for r, theirs in (close + other)[:25]:
            print(f"    {r['ats']}:{r['slug']:<28} ATS says {r['name']!r:<38} list says {theirs!r}")
    print()


def show_shared_names(rows):
    live = [r for r in rows if r["status"] == "live" and r["name"]]
    groups = defaultdict(list)
    for r in live:
        groups[company_key(r["name"])].append(r)
    shared = {k: g for k, g in groups.items() if len(g) > 1}
    cross = {k: g for k, g in shared.items() if len({r["ats"] for r in g}) > 1}
    same = {k: g for k, g in shared.items() if k not in cross}
    print("## One company name, several live boards\n")
    print(f"{len(shared)} names cover {sum(len(g) for g in shared.values())} live boards "
          f"({len(cross)} across two ATSs, {len(same)} within one).")
    print("Jobs from these boards share a company key, so dedup may merge same-titled jobs across them.\n")
    for label, gs in (("across ATSs", cross), ("within one ATS", same)):
        print(f"{label}:")
        for k, g in sorted(gs.items(), key=lambda kv: -len(kv[1]))[:15]:
            cells = ", ".join(f"{r['ats']}:{r['slug']} ({r['jobs'] if r['jobs'] is not None else '?'})" for r in g)
            print(f"  {g[0]['name']!r:<32} {cells}")
    print()


def show_suspicious(rows):
    live = [r for r in rows if r["status"] == "live"]
    odd = [r for r in live if r["name"] and SUSPICIOUS.search(r["name"])]
    odd += [r for r in live if not r["name"]]
    print(f"## Live boards whose name looks wrong or is missing ({len(odd)})\n")
    for r in odd[:30]:
        print(f"  {r['ats']}:{r['slug']:<30} {r['name']!r:<40} jobs={r['jobs']}")
    print()


def show_sample(rows, n, seed):
    live = sorted((r for r in rows if r["status"] == "live"), key=lambda r: r["key"])
    rng = random.Random(seed)
    print(f"## {n} random live boards to read by hand (seed {seed})\n")
    for r in sorted(rng.sample(live, min(n, len(live))), key=lambda r: r["ats"]):
        print(f"  {r['ats']}:{r['slug']:<30} {r['name']!r:<36} {r['website'] or '':<34} jobs={r['jobs']}")
    print()


def cmd_boards(a):
    rows = boards(a.db)
    fd_names = {}
    if a.fd:
        for c in json.load(open(a.fd)):
            slug = c["slug"] if c["ats"] == "lever" else c["slug"].lower()
            fd_names[(c["ats"], f"{c['ats']}:{slug}")] = c["name"]
    show_funnel(rows)
    show_finders(rows)
    show_names(rows, fd_names)
    show_shared_names(rows)
    show_suspicious(rows)
    show_sample(rows, a.sample, a.seed)


def cmd_sample(a):
    rows = [r for r in boards(a.db) if r["status"] == "live"]
    rng = random.Random(a.seed)
    print("version = 1\n")
    for ats in ATS:
        rs = sorted((r for r in rows if r["ats"] == ats), key=lambda r: r["key"])
        for r in rng.sample(rs, min(a.per_ats, len(rs))):
            print(f"[[sources]]\nid = {json.dumps(ats + ':' + r['slug'])}")
            if r["name"]:
                print(f"company = {json.dumps(r['name'])}")
            print()


def cmd_richness(a):
    totals = defaultdict(Counter)
    fields = ("scope", "timezone", "pay", "payUsd", "postedAt", "description")
    for arg in a.evals:
        label, path = arg.split("=", 1)
        for s in json.load(open(path))["sources"]:
            ats = s["source"].split(":")[0]
            g = totals[f"{label} {ats}" if ":" in s["source"] else s["source"]]
            n = s["jobs"]
            g["boards"] += 1
            g["jobs"] += n
            g["unmapped"] += s["locationUnmapped"]
            g["biggest"] = max(g["biggest"], n)
            for f in fields:
                g[f] += s["coverage"][f] * n
    print(f"{'group':<24}{'boards':>7}{'jobs':>7}{'biggest':>8}" + "".join(f"{f:>12}" for f in fields) + f"{'unmapped':>10}")
    for name, g in sorted(totals.items()):
        n = g["jobs"]
        print(f"{name:<24}{g['boards']:>7}{n:>7}{g['biggest']:>8}" + "".join(f"{pct(g[f], n):>12}" for f in fields)
              + f"{pct(g['unmapped'], n):>10}")


def main():
    p = argparse.ArgumentParser(prog="discovery_eval.py")
    sub = p.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("boards")
    b.add_argument("db")
    b.add_argument("--fd")
    b.add_argument("--sample", type=int, default=30)
    b.add_argument("--seed", type=int, default=1)
    b.set_defaults(run=cmd_boards)
    s = sub.add_parser("sample")
    s.add_argument("db")
    s.add_argument("--per-ats", type=int, default=40)
    s.add_argument("--seed", type=int, default=1)
    s.set_defaults(run=cmd_sample)
    r = sub.add_parser("richness")
    r.add_argument("evals", nargs="+", metavar="LABEL=EVAL_JSON")
    r.set_defaults(run=cmd_richness)
    a = p.parse_args()
    a.run(a)


if __name__ == "__main__":
    sys.exit(main())
