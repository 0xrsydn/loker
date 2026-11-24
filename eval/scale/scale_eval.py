#!/usr/bin/env python3

import argparse
import json
import os
import shutil
import sqlite3
import statistics
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

HERE = Path(__file__).resolve().parent
QUERIES = [q["q"] for q in json.loads((HERE.parent / "search" / "queries.json").read_text())]

FILTERS = [
    [],
    ["--country", "ID", "--tz", "7"],
    ["--country", "ID", "--tz", "7", "--min-verdict", "maybe", "--compact"],
    ["--country", "DE", "--tz", "1", "--min-verdict", "yes"],
    ["--country", "US", "--role", "engineering", "--titles"],
    ["--seniority", "senior,lead", "--compact"],
    ["--years", "3", "--country", "BR", "--tz", "-3"],
    ["--posted-within", "30", "--titles"],
    ["--role", "data,forward_deployed", "--posted-within", "365"],
    ["--offset", "40", "--limit", "20"],
    ["--any-word", "--titles", "--limit", "100"],
]

LISTINGS = [
    ["--titles"],
    ["--country", "ID", "--tz", "7", "--min-verdict", "maybe", "--titles", "--limit", "100"],
    ["--country", "US", "--titles", "--limit", "100", "--offset", "300"],
    ["--posted-within", "7", "--compact"],
    ["--role", "engineering", "--seniority", "junior,intern", "--titles"],
]


def cases(n):
    out = [[] + f for f in LISTINGS]
    i = 0
    while len(out) < n:
        q = QUERIES[i % len(QUERIES)]
        f = FILTERS[(i * 7 + i // len(QUERIES)) % len(FILTERS)]
        out.append(q.split() + f)
        i += 1
    return out[:n]


def search(binary, home, args):
    env = dict(os.environ, LOKER_HOME=str(home), RUST_LOG="error")
    start = time.perf_counter()
    r = subprocess.run([binary, "search", *args, "--json"], env=env, capture_output=True, text=True)
    took = time.perf_counter() - start
    if r.returncode != 0:
        return took, {"error": r.stderr.strip()[-300:]}
    return took, json.loads(r.stdout)


def synthesize(src, out, copies):
    shutil.copyfile(src, out)
    db = sqlite3.connect(out)
    db.execute("PRAGMA journal_mode=WAL")
    n = db.execute("SELECT count(*) FROM canonical_jobs").fetchone()[0]
    for k in range(1, copies + 1):
        s = f"_{k}"
        db.execute(
            "INSERT INTO canonical_jobs (id, job, content_hash, first_seen, last_seen, status, closed_at, role, role_stamp) "
            "SELECT id || ?, job, content_hash, first_seen, last_seen, status, closed_at, role, role_stamp "
            "FROM canonical_jobs WHERE instr(id, '_') = 2 AND length(id) = 12",
            (s,),
        )
        db.execute(
            "INSERT INTO jobs_fts (job_id, title, company, description) "
            "SELECT job_id || ?, title, company, description FROM jobs_fts "
            "WHERE instr(job_id, '_') = 2 AND length(job_id) = 12",
            (s,),
        )
        db.execute(
            "INSERT INTO source_jobs (source_id, source_job_id, source_url, canonical_job_id, raw_data, fetched_at, "
            "source_kind, job, content_hash, company_key, title_key, first_seen, status, gone_at, gone_reason, expires_at) "
            "SELECT source_id, source_job_id || ?, source_url, canonical_job_id || ?, '', fetched_at, source_kind, NULL, "
            "content_hash, company_key, title_key, first_seen, status, gone_at, gone_reason, expires_at "
            "FROM source_jobs WHERE instr(canonical_job_id, '_') = 2 AND length(canonical_job_id) = 12",
            (s, s),
        )
        db.commit()
        print(f"copy {k}/{copies}", file=sys.stderr)
    total = db.execute("SELECT count(*) FROM canonical_jobs").fetchone()[0]
    db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    db.close()
    print(f"{n} jobs -> {total}")


def summary(xs):
    xs = sorted(xs)
    p95 = xs[min(len(xs) - 1, int(round(0.95 * (len(xs) - 1))))]
    return f"median {statistics.median(xs) * 1000:.0f} ms, p95 {p95 * 1000:.0f} ms, max {xs[-1] * 1000:.0f} ms"


def run(a):
    work = Path(a.work)
    work.mkdir(parents=True, exist_ok=True)
    homes = {}
    for side in ("old", "new"):
        home = work / side
        if home.exists():
            shutil.rmtree(home)
        home.mkdir()
        shutil.copyfile(a.db, home / "loker.db")
        homes[side] = home
    jobs = sqlite3.connect(a.db).execute("SELECT count(*) FROM canonical_jobs WHERE status = 'active'").fetchone()[0]
    print(f"{jobs} active jobs in {a.db}")
    took, _ = search(a.new, homes["new"], ["--limit", "1", "--titles"])
    print(f"new binary, first open (migrations and derived columns): {took:.1f} s")
    todo = cases(a.cases)
    new_times, new_out = [], []
    for args in todo:
        runs = [search(a.new, homes["new"], args) for _ in range(a.runs)]
        new_times.append(statistics.median(t for t, _ in runs))
        new_out.append(runs[0][1])
    print(f"new binary, {len(todo)} cases x {a.runs} runs: {summary(new_times)}")
    slowest = sorted(zip(new_times, todo), reverse=True)[:5]
    for t, args in slowest:
        print(f"  {t * 1000:6.0f} ms  {' '.join(args)}")
    errors = [(args, o["error"]) for args, o in zip(todo, new_out) if "error" in o]
    for args, e in errors:
        print(f"  ERROR {' '.join(args)}: {e}")
    if a.skip_old:
        return 1 if errors else 0
    if a.old_cases:
        step = max(1, len(todo) // a.old_cases)
        picked = list(range(0, len(todo), step))[: a.old_cases]
        todo = [todo[i] for i in picked]
        new_out = [new_out[i] for i in picked]
    with ThreadPoolExecutor(a.parallel) as ex:
        old = list(ex.map(lambda args: search(a.old, homes["old"], args), todo))
    print(f"old binary, {len(todo)} cases x 1 run ({a.parallel} at a time): {summary([t for t, _ in old])}")
    same = rerun = 0
    for args, (_, o), n in zip(todo, old, new_out):
        if o == n:
            same += 1
            continue
        _, n = search(a.new, homes["new"], args)
        _, o = search(a.old, homes["old"], args)
        if o == n:
            same += 1
            rerun += 1
            continue
        print(f"  DIFFERENT: {' '.join(args)}")
        for key in ("total", "fullMatches", "count", "nextOffset"):
            if o.get(key) != n.get(key):
                print(f"    {key}: old {o.get(key)} new {n.get(key)}")
        oj, nj = o.get("jobs", []), n.get("jobs", [])
        for i, (x, y) in enumerate(zip(oj, nj)):
            if x != y:
                print(f"    first differing job at {i}: old {json.dumps(x)[:300]}")
                print(f"                                new {json.dumps(y)[:300]}")
                break
    print(f"identical results: {same}/{len(todo)} ({rerun} only when run back to back: a date cutoff moved between runs)")
    return 0 if same == len(todo) and not errors else 1


def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("synthesize")
    s.add_argument("src")
    s.add_argument("out")
    s.add_argument("copies", type=int)
    r = sub.add_parser("run")
    r.add_argument("--old", required=True)
    r.add_argument("--new", required=True)
    r.add_argument("--db", required=True)
    r.add_argument("--work", default="eval/scale/work")
    r.add_argument("--cases", type=int, default=80)
    r.add_argument("--runs", type=int, default=3)
    r.add_argument("--parallel", type=int, default=6)
    r.add_argument("--skip-old", action="store_true")
    r.add_argument("--old-cases", type=int, default=0)
    a = p.parse_args()
    if a.cmd == "synthesize":
        synthesize(a.src, a.out, a.copies)
    else:
        sys.exit(run(a))


if __name__ == "__main__":
    main()
