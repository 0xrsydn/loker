# Scale eval

Runs an older and a newer `loker` binary over copies of one database. It
checks that they return the same results for every case, and times both.
Run it by hand when changing how search or ingest works (`store.rs`,
`api.rs`, `derived.rs`, a description reader, a migration with indexes); it
isn't part of CI. A faster
search that returns different jobs is a regression, so parity comes first.

```sh
cargo build --release                                     # the new binary
git worktree add /tmp/old <commit> && (cd /tmp/old && cargo build --release)
python3 scale_eval.py synthesize SRC.db OUT.db 15         # optional: 16x the jobs of SRC
python3 scale_eval.py run --old /tmp/old/target/release/loker \
  --new ../../target/release/loker --db DB [--cases 80] [--runs 3] \
  [--parallel 6] [--old-cases N] [--skip-old]
```

`run` copies the database into `work/old` and `work/new`. It times the new
binary's first open, because that open runs migrations and fills derived
columns. It then runs each case `--runs` times with the new binary, and once
with the old binary, `--parallel` at a time. `--old-cases N` runs only N
cases with the old binary, spread evenly, for a database where the old binary
takes minutes per case. The cases are the 38 queries of
`eval/search/queries.json` crossed with filters, plus searches with no words.
Each case's `--json` output must be identical.

A case with `--posted-within` can differ by one job when a job's posting date
falls between the old run and the new one. When outputs differ, the case is
run again on both binaries back to back. If they then match, it is counted
separately as only matching back to back.

`synthesize` copies every canonical job, its full-text row and its listings
with an `_k` id suffix. Jobs repeat, so each query matches exactly 16 times as
many jobs. That makes it a worst case for growth, not a realistic text
distribution.

## Results

Run on 2026-11-24 on an 8-core Xeon E5-2673 v3, all on one machine. Old is
3791c81. New is the derived-columns change (`perf: search filters in SQL`).
The old binary ran 4 to 6 cases at once, so its times include some
contention.

| database | jobs | identical | new median | new p95 | new max | old median | old max | first open (new) |
|---|---|---|---|---|---|---|---|---|
| `rich` (ingest of rich boards) | 6,742 | 80/80 | 54 ms | 87 ms | 91 ms | 6.7 s | 22 s | 3.8 s |
| real ingest, 360 boards | 19,066 | 80/80 | 107 ms | 183 ms | 199 ms | 56 s | 206 s | 10.5 s |
| that, synthesized 16x (6.3 GB) | 305,056 | 4/4 old cases | 729 ms | 1.68 s | 1.75 s | 19 min | 90 min | 373 s |

The old binary's 90 minutes at 305k is a search with no words listing every
job. It re-sorted all matches for every batch of 200, and read visa and years
from each description, so its time grew with the square of the matches. In an
earlier run of the 19k database, one of 80 cases differed by one job,
because of the `--posted-within` clock edge described above.

The new binary's slowest cases at 305k match most of the jobs. A role filter
or a query matching about 291k jobs costs, measured one part at a time:
- the full-text match: 76 ms;
- joining and counting it: 330 ms;
- the all-words rank: 530 ms;
- the role JSON filter: 800 ms;
- the page's bm25 sort: 730 ms.

The count and the page are two passes. ARCHITECTURE.md ("When to change the
design") lists what to do once that matters on a real database.

The first open after an upgrade costs about 1.2 ms per job, once. That is
6 minutes at 305k. A later change to the derivation rules, which changes
`derived::STAMP`, costs the same again; after the faster readers below, about
0.3 ms a job (5.8 s at 19k).

## Ingest

`ingest` replays a real ingest offline. It exports every stored raw item of
a database, feeds them board by board through `job_from_raw` and
`Store::ingest` (the `replay` example in `loker-sources`) into a fresh
database, twice: a first ingest, then a re-ingest of the same jobs. It does
this with an old and a new `replay` build and compares the two databases row
by row: canonical jobs (JSON and derived columns), listings and their links,
and full-text rows. Timestamps are left out.

```sh
cargo build --release --example replay                     # in each checkout
python3 scale_eval.py ingest --old OLD/target/release/examples/replay \
  --new ../../target/release/examples/replay --db DB [--passes 2] [--keep-old]
RUST_LOG=sqlx::query=debug replay OUT.db 1 < items.jsonl   # each statement's time
```

`--keep-old` reuses the old database from the last run. On 2026-11-24, real
ingest of 360 boards (19,066 jobs), same machine as above:

| | first ingest | re-ingest |
|---|---|---|
| old (1f53655) | 1,304 s (transform 104 s, store 1,199 s) | 179 s (101 s, 78 s) |
| new | 84 s (16 s, 68 s) | 22.5 s (12 s, 10 s) |

All three tables were identical. The old store's time grew with the whole
database: a full-text scan for every new job, a scan of every live listing
for every changed one, and a read of every live row per board for the
expired-or-stale check. The description readers had to run the regex
crate's slowest engine, because Unicode `\b` on non-ASCII text rules out its
DFA; each now skips text that can't match, then runs the exact pattern.
Over the 19k descriptions they took 132 s and now take 11 s, with
byte-identical output (`readers` example):

| reader | before | after |
|---|---|---|
| pay | 69–78 s | 2.7 s |
| stated eligibility | 41–46 s | 3 s |
| open to wider | 4.8 s | 1.8 s |
| visa | 8.5 s | 1.6 s |
| experience | 4.6 s | 2.1 s |
