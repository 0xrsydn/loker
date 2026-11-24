# AGENTS.md

A map for coding agents working on loker. It points to where things are and
lists what's easy to get wrong; it doesn't tell you how to work. Use your
judgment, and when this file and the code disagree, trust the code and fix
this file.

**loker** is a self-hosted job aggregator in Rust: sources are ingested into
SQLite, deduplicated across sources, tagged with eligibility (who can apply
from where), and searched over a CLI and an MCP server.

## Read first

| File | For |
|---|---|
| [README.md](README.md) | What loker does and how to run it |
| [ARCHITECTURE.md](ARCHITECTURE.md) | The pipeline, where code goes, the rules and trade-offs, checklists for a new source or field, when the design should change |

## Map

```
crates/loker-core/       domain and rules, no network
  src/model.rs           NewJob (one listing), Job (search result), Eligibility, Pay, Visa, Experience
  src/normalize.rs       shared rules: match keys, URLs, pay, eligibility from text
  src/pay_text.rs        pay read from a description, for sources with no pay fields
  src/visa_text.rs       visa sponsorship read from a description, when a job is read
  src/experience_text.rs years of experience required, read from a description when a job is read
  src/eligibility_text.rs where a remote job may be done from, read from its description at ingest
  src/country.rs         country, state, province and city names -> ISO codes; region tables
  src/dedup.rs           which listings are one job; no IO
  src/derived.rs         what search filters on, derived when a job is written; its rules stamp
  src/board.rs           which Ashby, Greenhouse or Lever board a URL names; board keys
  src/verdict.rs         can this viewer apply? (country, timezone)
  src/role.rs            role vocabulary: function hierarchy, modifiers, JobRole
  src/classify.rs        which role a job is: title, then category, then description
  src/store.rs           SQLite: ingest, liveness, search, eval
  src/api.rs             the typed v1 contract every interface returns
  src/config.rs          $LOKER_HOME/config.toml
  migrations/            SQL migrations, embedded at compile time
  tests/                 dedup, lifecycle, search, storage
crates/loker-sources/    adapters: fetch + transform per source, fetch_batch, verify; discover, commoncrawl, websearch
crates/loker/            App facade, CLI (main.rs), MCP (mcp.rs), tests/e2e.rs
schema/v1/               JSON Schemas generated from the api types
contrib/systemd/         unattended ingest service + timer
eval/search/             manual search-ranking eval (Python, LLM-judged); not in CI
eval/agent/              manual end-to-end agent eval (Pi + DeepSeek over MCP); not in CI
eval/role/               manual role-classifier eval against LLM-labelled jobs; not in CI
eval/pay/                manual eval of pay read from descriptions, with a holdout; not in CI
eval/visa/               manual eval of visa sponsorship read from descriptions; not in CI
eval/experience/         manual eval of required years read from descriptions; not in CI
eval/eligibility/        manual eval of per-viewer verdicts, rebuilt from stored raw items; not in CI
eval/discovery/          manual eval of discovered boards: real, named right, how rich; not in CI
eval/scale/              manual eval of search at scale: same results as an older binary, latency; not in CI
eval/golden/             24 jobs checked by hand, blind: rulings and how far the labels hold
lint/                    comment ratchet: no new comments, the count only goes down
```

## Build and check

The toolchain comes from the Nix dev shell (`direnv allow` or `nix develop`).
The system `rustc` may not work; run cargo inside the shell.

```sh
cargo test                       # all tests, offline, a few seconds
cargo clippy --all-targets -- -D warnings
cargo fmt                        # CI fails on unformatted code
python3 lint/comment_ratchet.py --check   # no new comments (CI runs it too)
nix flake check                  # what CI runs: release build, tests, clippy, rustfmt, comments
cargo run -- <command>           # ingest, search, verify, sources eval, stats, schema, mcp
```

Inside the repo, `.envrc` sets `LOKER_HOME=./.loker-dev`, so development never
touches real data. Use a scratch `LOKER_HOME` for experiments you don't want
in that database.

## Easy to get wrong

- **Contract changes:** after changing a type in `api.rs` (or anything it
  contains), regenerate with `cargo run -- schema --out schema/v1` and commit
  the diff. A test fails until you do. Keep changes additive within v1.
- **Migrations:** add a new numbered file. Never edit one that has shipped;
  sqlx checks their checksums.
- **Search filters:** a filter runs in SQL over a derived column
  (`derived.rs`, `store.rs`), so a new one needs a column written in
  `DerivedCols`, a place in both covering indexes, and a case in
  `sql_filters_keep_exactly_the_jobs_the_rules_keep`.
- **SQL strings:** sqlx 0.9 rejects SQL built with `format!`. Use string
  literals, or `concat!` inside a `macro_rules!` as `store.rs` does.
- **Stored job JSON:** `NewJob` is stored and hashed. A new field must be
  optional and skipped when empty; don't rename fields without a migration
  (ARCHITECTURE.md rule 5).
- **Live sources are someone else's servers.** Prefer the e2e stub
  (`LOKER_FETCH_BASE` points every adapter at it) and `loker verify` (one page
  per source) over repeated live ingests.
- **No comments:** say it in names, types and tests; a decision goes in
  ARCHITECTURE.md, README.md or an eval README. `lint/comment_ratchet.py`
  fails CI when a file gains a comment line and when one is deleted without
  `--update`. Doc comments on `JsonSchema` and clap types are output (schema
  descriptions, `--help`) and are not counted. `/delete-comment` runs a pass.
- **Tests stay lean:** integration tests over real behaviour, a table-driven
  test for a pure rule, and a test to pin a fixed bug. No per-function test
  suites and no snapshot tests.

## Changes and commits

- One commit per scoped change, Conventional Commits style (`feat:`, `fix:`,
  `refactor(core):`, `test:`, `docs:`, `ci:`, `build:`). The body says why.
- For a refactor, show that behaviour didn't change. The existing tests should
  pass unedited; for dedup or store changes, comparing databases from the old
  and new binaries is a good check.
- Update README.md or ARCHITECTURE.md in the same commit when behaviour or a
  rule changes.
- GitHub is the primary repo; the Forgejo copy is a read-only mirror.
