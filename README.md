# loker

A self-hosted job aggregator that knows **who can actually apply from where**.
*Loker* is Indonesian slang for job openings (*lowongan kerja*).

loker pulls listings from job boards and company ATS boards into one SQLite file,
records each job's eligibility (worldwide, listed countries, required or preferred
time zones), and answers searches from your point of view: "open to someone in
Indonesia at UTC+7", not just "remote".

> Status: early foundation. Sources: Himalayas, Jobicy, Remote OK, and any
> company's Greenhouse or Ashby board. CLI and MCP (stdio). REST and the web UI
> come next.

## Try it

```sh
direnv allow                                  # or `nix develop`: toolchain, sqlite, litestream, secretspec
cargo run -- ingest himalayas --country ID    # jobs open to Indonesia (worldwide + ID listed)
cargo run -- search engineer --country ID --tz 7
cargo run -- ingest ashby:clera                # one company's board: ashby:<slug>, greenhouse:<slug>
cargo run -- ingest remoteok                   # also: jobicy
cargo run -- sources eval                     # coverage, overlap, blocked merges, conflicts
cargo run -- stats
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

### As an MCP server

`loker mcp` serves the tools over stdio. For Claude Code:

```sh
claude mcp add loker -- loker mcp
```

Tools: `search_jobs` (query, `country`, `tz`, `limit`; each job carries an
eligibility verdict with a reason), `sources_eval` and `stats`. Bad input comes back as a tool
error the model can read and fix, such as "unknown country `Narnia`".

Build the binary reproducibly with `nix build` (output in `./result/bin/loker`).

## Layout

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
| `loker-core` | Domain model, eligibility verdicts, `normalize` (match keys, URLs, pay, locations), SQLite store with cross-source dedup, and `api`: the typed v1 contract plus the store-only operations. No transport code. |
| `loker-sources` | Adapters (`fetch` + `transform` each): Himalayas, Jobicy, Remote OK, Greenhouse, Ashby; `Source` dispatch and `ingest`, the one fetch → transform → store path |
| `loker` | `App`, the handle every adapter holds; the CLI (`main.rs`) and MCP adapter (`mcp.rs`) |

`nix flake check` builds the release binary, runs the tests (one offline test
file for the dedup rules) and runs clippy with warnings denied.

## Repository

Development, issues and pull requests: https://github.com/0xrsydn/loker.
Read-only mirror: https://git.rasyidanaf.com/0xrsydn/loker.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
