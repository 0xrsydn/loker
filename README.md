# loker

A self-hosted job aggregator that knows **who can actually apply from where**.
*Loker* is Indonesian slang for job openings (*lowongan kerja*).

loker pulls listings from job boards and company ATS boards into one SQLite file,
records each job's eligibility (worldwide, listed countries, required or preferred
time zones), and answers searches from your point of view: "open to someone in
Indonesia at UTC+7", not just "remote".

> Status: early foundation. One source (Himalayas); CLI and MCP (stdio).
> REST, more sources and the web UI come next.

## Try it

```sh
direnv allow                                  # or `nix develop`: toolchain, sqlite, litestream, secretspec
cargo run -- ingest himalayas --country ID    # jobs open to Indonesia (worldwide + ID listed)
cargo run -- search engineer --country ID --tz 7
cargo run -- stats
```

```
j_6b78581675  Senior Engineer, Service Delivery — dell
    yes: ID listed, inside UTC+7..+9 · pay n/a
```

Data lives in `~/.loker/loker.db` (override with `LOKER_HOME`; inside the repo,
`.envrc` points it at `./.loker-dev` so development never touches your real data).
Running `ingest` twice is safe: unchanged jobs are only marked as seen again.

### As an MCP server

`loker mcp` serves the tools over stdio. For Claude Code:

```sh
claude mcp add loker -- loker mcp
```

Tools: `search_jobs` (query, `country`, `tz`, `limit`; each job carries an
eligibility verdict with a reason) and `stats`. Bad input comes back as a tool
error the model can read and fix, such as "unknown country `Narnia`".

Build the binary reproducibly with `nix build` (output in `./result/bin/loker`).

## Layout

Every operation is defined once and every interface calls it the same way:

```
CLI flags ──┐                                     ┌── loker_core::api (search_jobs, stats)
MCP args  ──┼─► params (api::*Params) ─► App ─────┤
REST body ──┘   (later)                           └── loker_sources::ingest
```

Params and results are plain serde + schemars types in `loker_core::api` (contract
version 1). Adapters only parse input and render output, so `loker search --json`
prints exactly what the MCP tool returns as structured content.

| Crate | Role |
|---|---|
| `loker-core` | Domain model, eligibility verdicts, country mapping, SQLite store, and `api`: the typed v1 contract plus the store-only operations. No transport code. |
| `loker-sources` | Source adapters (Himalayas so far) and `ingest`, the one fetch → transform → store path |
| `loker` | `App`, the handle every adapter holds; the CLI (`main.rs`) and MCP adapter (`mcp.rs`) |

`nix flake check` builds the release binary and runs clippy with warnings denied.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
