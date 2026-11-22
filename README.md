# loker

A self-hosted job aggregator that knows **who can actually apply from where**.
*Loker* is Indonesian slang for job openings (*lowongan kerja*).

loker pulls listings from job boards and company ATS boards into one SQLite file,
records each job's eligibility (worldwide, listed countries, required or preferred
time zones), and answers searches from your point of view: "open to someone in
Indonesia at UTC+7", not just "remote".

> Status: early foundation. Sources: Himalayas, Jobicy, Remote OK, We Work
> Remotely, Workable (opt-in), Y Combinator's board, and any company's
> Greenhouse, Ashby or Lever board. CLI and MCP (stdio). REST and the web UI
> come next.

## Try it

```sh
direnv allow                                  # or `nix develop`: toolchain, sqlite, litestream, secretspec
cargo run -- config init --country ID         # starter sources, with Himalayas and Workable searching Indonesia
cargo run -- ingest --all                     # fetch them all (without a config: the starter sources)
cargo run -- doctor                           # setup, database, each source's last fetch; --live fetches too
cargo run -- search engineer --country ID --tz 7
cargo run -- search backend --country ID --tz 7 --min-verdict yes --compact   # only jobs you can take, short
cargo run -- search golang rust kotlin        # every word first, then partial matches
cargo run -- search developer --seniority junior,intern   # levels from the title or source
cargo run -- search backend --years 3   # drops jobs asking for more than 3 years; keeps jobs that state none
cargo run -- search --role ai,data --country ID --compact      # by role, any level; `loker roles` lists them
cargo run -- roles enrich                      # optional: Jev classifies roles (needs [providers.jev] and OPENROUTER_API_KEY)
cargo run -- search --country ID --tz 7 --min-verdict yes --titles --limit 100   # everything you can apply to, one line each
cargo run -- get j_6b78581675                  # full details for ids from a search
cargo run -- ingest ashby:clera                # one company's board: ashby:<slug>, greenhouse:<slug>, lever:<site>
cargo run -- ingest remoteok                   # also: jobicy, weworkremotely
cargo run -- ingest workable --country PH       # Workable's search, remote jobs; opt-in (not a published API)
cargo run -- ingest ycombinator --pages 50      # YC's whole board: a sitemap, then one request per company (~2 h)
cargo run -- search --country ID --offset 20   # next page (each page reports its next offset)
cargo run -- sources eval                     # coverage, overlap, blocked merges, conflicts, closed jobs
cargo run -- verify                           # live check of every configured source; non-zero exit on failure
cargo run -- stats
```

### Running it unattended

List your sources once in `$LOKER_HOME/config.toml` (`loker config init` writes
a starting one; `loker config check` validates it):

```toml
version = 1

[[sources]]
id = "himalayas"
country = "ID"         # only jobs open to Indonesia
pages = 5

[[sources]]
id = "ashby:clera"
company = "Clera"      # display name; Ashby and Lever only give the slug

[[sources]]
id = "greenhouse:binance"
remote = true          # remote-first: listed countries are where you can work from

[[sources]]
id = "ycombinator"
pages = 50             # one request per company page; 50 covers the whole board

[[sources]]
id = "remoteok"
enabled = false        # listed but skipped
```

config.toml holds intent (which sources, with what parameters); what
happened lives in the database. The file is versioned, and an unknown key is
an error, so a typo never silently disables a source.
`loker ingest --all` fetches every enabled source, several at once: at most
4 fetches in flight to one API host (all Greenhouse boards share one), started
at least 250 ms apart, so hundreds of company boards take minutes, not hours.
The report lists sources in config order. One failing source doesn't stop the
others, and the exit status is non-zero if any failed.
`loker verify` checks the same sources with the same parameters, the same way.
`contrib/systemd/` has a oneshot service (ingest, then verify) and a timer
that runs it every 6 hours, with up to 20 minutes of random delay so many
installs don't hit the sources at the same moment; a run missed while the
machine was off happens at the next boot. The service file says how to
install it as a user unit.

**Secrets.** None is required: without them loker runs fully offline.
Optional providers read their key from the environment, never from
config.toml, and `secretspec.toml` declares them, so any secretspec
provider (keyring, 1Password, a file, ...) can supply them:

```toml
[providers.jev]   # in config.toml: Jev classifies job roles; needs OPENROUTER_API_KEY
```

```sh
secretspec run -- loker ingest --all   # in the repo; elsewhere add --file /path/to/secretspec.toml
```

```
j_6b78581675  Senior Engineer, Service Delivery — dell
    yes: ID listed, inside UTC+7..+9 · pay n/a
```

Data lives in `~/.loker/loker.db` (override with `LOKER_HOME`; inside the repo,
`.envrc` points it at `./.loker-dev` so development never touches your real data).
Running `ingest` twice is safe: unchanged jobs are only marked as seen again.

### One job, many listings

The same role often appears on an aggregator and on the company's own board.
Each listing is kept per source; listings with the same company and title
(normalized: "Lemon.io, Inc." = "lemon.io") become one job, unless one source
lists that title twice (e.g. one per city), in which case nothing is merged.
The outcome depends only on what is stored, never on ingest order. A merged job
takes its link and text from the employer board, its eligibility from the most
confident source, and lists every source it came from (`sources`) so
aggregators that require attribution (Remote OK, Jobicy) get their link.

### Who can apply

Each job records where it is open (worldwide, countries, regions such as APAC
or EMEA, or on-site), and every search with `--country` turns that into a
verdict for you: `yes`, `maybe`, `unknown`, or `no` (dropped). Regions resolve
to their countries when you search, so "APAC" is a yes from Indonesia and a no
from Brazil. Regions mean what job posts mean by them, not geography: APAC is
East, Southeast and South Asia plus Oceania; EMEA is Europe, the Middle East
and Africa; LATAM is Mexico, Central America, the Latin Caribbean and South
America. Türkiye and Egypt count in two blocks. A posting that lists places but not whether it is remote (most
Greenhouse boards) is still tied to them: `no` from elsewhere, `maybe` from
inside, since it may be on-site. For a remote-first company whose locations
are where you can be based, `remote = true` on its board in config.toml makes
those countries a plain list: `yes` from inside. The flag only fills in what a
posting leaves unknown; a posting that says on-site, hybrid or remote keeps
it.

### Jobs close

A listing closes when a complete fetch of its source no longer has it (a
company's Greenhouse, Ashby or Lever board), when the source's own expiry passes
(Himalayas), or after 30 days unseen (the backstop for feeds that only return
their newest jobs). It reopens when seen again. A job stays active while any
of its listings is live. A fetch that would close more than half of a source's
listings closes nothing and reports them as `held`.

### Checking sources

`loker verify` fetches each source live, stores nothing, and checks what
every adapter must keep true: items returned, parse rate, unique ids, title,
company and URL present, ids for closures, dates. It is how an upstream API
change shows up. With no names it checks the sources in config.toml, each with
its configured parameters.

### Finding company boards

Greenhouse, Ashby and Lever publish no list of their boards, so loker finds
them. `loker discover` reads board URLs or ids, stores each board once, then
asks each new board's API whether it exists: `live` (open jobs), `empty`, or
`missing`. Each board is stored under a key, so two spellings of one board
count once: Ashby and Greenhouse slugs ignore case, Lever's don't
(`lever:Palantir` is not `lever:palantir`). Slugs keep their spaces (`ashby:Hippocratic AI`),
and a check stores the spelling the ATS itself uses. A board whose check
gets no answer is asked again on the next run.

```sh
cargo run -- discover                     # Common Crawl's newest crawl, then check every new board
cargo run -- discover --check 0           # store what the crawl names, ask later
cargo run -- discover --from boards.txt   # board URLs or ids, one per line
```

By default it reads [Common Crawl](https://commoncrawl.org)'s URL index, a
free monthly crawl of the web, for URLs on the three ATS hosts (EU hosts
included), without its rate-limited query server. Each crawl is read
once; `--again` reads it anew. CC-MAIN-2026-43 named 5,582 boards in 31 s
(1,079 Ashby, 4,173 Greenhouse, 330 Lever). A crawl misses newer and smaller
companies: it had 288 of the 509 boards on forwarddeployed.fun, whose list
loads as a file. Checking those 509 took 113 s: 503 live, 6 empty.

### As an MCP server

`loker mcp` serves the tools over stdio. For Claude Code:

```sh
claude mcp add loker -- loker mcp
```

Tools: `search_jobs` (query, `anyWord`, `country`, `tz`, `limit`, `offset`,
`minVerdict`, `seniority`, `experienceYears`, `role`, `view`; each job carries an eligibility verdict with a reason
and its role), `get_jobs`, `list_roles` (the role vocabulary), `sources_eval`, `verify_sources`
and `stats`. Bad input comes back as a tool
error the model can read and fix, such as "unknown country `Narnia`".
config.toml is read each time it is needed, so edits apply without
restarting the server.

For agents, the cheap flow is a compact search for jobs the viewer can apply
to, then full details for the picks:

```json
{"q": "backend", "country": "ID", "tz": 7, "minVerdict": "yes", "view": "compact"}
```

then `get_jobs` with the chosen `ids`. A compact job is about a third of the
size of a full one. On a real question ("remote backend jobs I can take from
Indonesia"), a DeepSeek Flash agent in Pi used 10 tool calls and read 43K
characters this way, against 20 calls and 177K before these existed.

Build the binary reproducibly with `nix build` (output in `./result/bin/loker`).

## Layout

[ARCHITECTURE.md](ARCHITECTURE.md) has the pipeline, the rules every change
follows, and when the design should change. Read it before changing the core or
adding a source. Coding agents start at [AGENTS.md](AGENTS.md).

Every operation is defined once and every interface calls it the same way:

```
CLI flags ──┐                                     ┌── loker_core::api (search_jobs, stats)
MCP args  ──┼─► params (api::*Params) ─► App ─────┤   (+ sources_eval)
REST body ──┘   (later)                           └── loker_sources::ingest
```

Params and results are plain serde + schemars types in `loker_core::api` (contract
version 1). Adapters only parse input and render output, so `loker search --json`
prints exactly what the MCP tool returns as structured content.

| Crate | Role |
|---|---|
| `loker-core` | Domain model, eligibility verdicts, `normalize` (match keys, URLs, pay, locations), `dedup` (which listings are one job and what that job says, with no IO), the SQLite store that loads rows for those rules and writes their answers, and `api`: the typed v1 contract plus the store-only operations. No transport code. |
| `loker-sources` | Adapters (`fetch` + `transform` each): Himalayas, Jobicy, Remote OK, We Work Remotely, Y Combinator, Greenhouse, Ashby, Lever; `Source` dispatch and `ingest`, the one fetch → transform → store path |
| `loker` | `App`, the handle every adapter holds; the CLI (`main.rs`) and MCP adapter (`mcp.rs`) |

The v1 contract is committed as JSON Schemas in `schema/v1/` (`loker schema
--out schema/v1` regenerates them); a test fails if the code and the committed
files disagree, so contract changes are always deliberate.

## Tests

`nix flake check` builds the release binary, runs every test, runs clippy
with warnings denied and checks formatting with rustfmt; CI runs it on every
push and pull request. CI reads build dependencies from a public, signed Nix
cache on Cloudflare R2 (alongside cache.nixos.org), and pushes to `main` refill
it (`.github/push-nix-cache.sh`). Reads are public, through the
cache.rasyidanaf.com gateway (a Cloudflare Worker), every path is signed by
`loker-ci-1`, and if the cache is unreachable Nix builds instead. The push
uploads only what cache.nixos.org doesn't serve: crane's compiled
dependencies and the vendored crate sources; per-commit outputs (the binary,
clippy, fmt, the filtered source) would never be reused. It needs an R2 token
with write access (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`),
`R2_ENDPOINT` and the `loker-ci-1` signing key (`NIX_SIGNING_KEY`); without
them it does nothing, so forks still pass. `NIX_CACHE_TO=file:///tmp/cache`
tries it without touching R2.

`lint/comment_ratchet.py --check` runs in `nix flake check` too: no file may
gain a comment line (see AGENTS.md).

- `crates/loker-core/tests/`: dedup rules (order independence, blocked merges,
  a table of `dedup::cluster` cases),
  lifecycle (closing, reopening, truncated fetches, expiry with a moved clock),
  search (eligible jobs behind many ineligible ones, offset paging) and storage
  (every JSON path the SQL reads still resolves).
- `crates/loker/tests/e2e.rs`: the real binary against a local stub serving one
  hand-written payload per adapter: ingest, idempotent re-runs, order
  independence, CLI = MCP parity, closing through a real fetch, `verify`,
  config-driven `ingest --all` (a failing source, a disabled one, a display
  name), and schema drift. `LOKER_FETCH_BASE` points every adapter at the stub (or at a
  caching proxy).

## Repository

Development, issues and pull requests: https://github.com/0xrsydn/loker.
Read-only mirror: https://git.rasyidanaf.com/0xrsydn/loker.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
