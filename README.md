# loker

A self-hosted job aggregator that knows **who can actually apply from where**.
*Loker* is Indonesian slang for job openings (*lowongan kerja*).

loker pulls listings from job boards and company ATS boards into one SQLite file,
records each job's eligibility (worldwide, listed countries, required or preferred
time zones), and answers searches from your point of view: "open to someone in
Indonesia at UTC+7", not just "remote".

> Status: early foundation. Sources: Himalayas, Jobicy, Remote OK, and any
> company's Greenhouse, Ashby or Lever board. CLI and MCP (stdio). REST and the web UI
> come next.

## Try it

```sh
direnv allow                                  # or `nix develop`: toolchain, sqlite, litestream, secretspec
cargo run -- ingest himalayas --country ID    # jobs open to Indonesia (worldwide + ID listed)
cargo run -- search engineer --country ID --tz 7
cargo run -- search backend --country ID --tz 7 --min-verdict yes --compact   # only jobs you can take, short
cargo run -- search golang rust kotlin        # every word first, then partial matches
cargo run -- search developer --seniority junior,intern   # levels from the title or source
cargo run -- get j_6b78581675                  # full details for ids from a search
cargo run -- ingest ashby:clera                # one company's board: ashby:<slug>, greenhouse:<slug>, lever:<site>
cargo run -- ingest remoteok                   # also: jobicy
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
id = "remoteok"
enabled = false        # listed but skipped
```

`loker ingest --all` fetches every enabled source in order; one failing source
doesn't stop the others, and the exit status is non-zero if any failed.
`loker verify` checks the same sources with the same parameters.
`contrib/systemd/` has a oneshot service (ingest, then verify) and a timer
that runs it every 6 hours; the service file says how to install it as a user
unit.

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
from Brazil. A posting that lists places but not whether it is remote (most
Greenhouse boards) is still tied to them: `no` from elsewhere, `maybe` from
inside, since it may be on-site. For a remote-first company whose locations
are where you can be based, `remote = true` on its board in config.toml makes
those countries a plain list: `yes` from inside.

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

### As an MCP server

`loker mcp` serves the tools over stdio. For Claude Code:

```sh
claude mcp add loker -- loker mcp
```

Tools: `search_jobs` (query, `anyWord`, `country`, `tz`, `limit`, `offset`,
`minVerdict`, `seniority`, `view`; each job carries an eligibility verdict with a reason), `get_jobs`,
`sources_eval`, `verify_sources` and `stats`. Bad input comes back as a tool
error the model can read and fix, such as "unknown country `Narnia`".

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
| `loker-sources` | Adapters (`fetch` + `transform` each): Himalayas, Jobicy, Remote OK, Greenhouse, Ashby, Lever; `Source` dispatch and `ingest`, the one fetch → transform → store path |
| `loker` | `App`, the handle every adapter holds; the CLI (`main.rs`) and MCP adapter (`mcp.rs`) |

The v1 contract is committed as JSON Schemas in `schema/v1/` (`loker schema
--out schema/v1` regenerates them); a test fails if the code and the committed
files disagree, so contract changes are always deliberate.

## Tests

`nix flake check` builds the release binary, runs every test, runs clippy
with warnings denied and checks formatting with rustfmt; CI runs it on every
push and pull request. CI reads build dependencies from a public, signed Nix
cache on Cloudflare R2 (alongside cache.nixos.org), and pushes to `main` refill
it (`.github/push-nix-cache.sh`).

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
