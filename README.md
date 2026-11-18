# loker

A self-hosted job aggregator that knows **who can actually apply from where**.
*Loker* is Indonesian slang for job openings (*lowongan kerja*).

loker pulls listings from job boards and company ATS boards into one SQLite file,
records each job's eligibility (worldwide, listed countries, required or preferred
time zones), and answers searches from your point of view: "open to someone in
Indonesia at UTC+7", not just "remote".

> Status: early foundation. One source (Himalayas), CLI only. MCP, REST, more
> sources and the web UI come next.

## Try it

```sh
nix develop                                   # Rust toolchain, sqlite, litestream, secretspec
cargo run -- ingest himalayas --country ID    # jobs open to Indonesia (worldwide + ID listed)
cargo run -- search engineer --country ID --tz 7
cargo run -- stats
```

```
j_6b78581675  Senior Engineer, Service Delivery — dell
    yes: ID listed, inside UTC+7..+9 · pay n/a
```

Data lives in `~/.loker/loker.db` (override with `LOKER_HOME`). Running `ingest`
twice is safe: unchanged jobs are only marked as seen again.

Build the binary reproducibly with `nix build` (output in `./result/bin/loker`).

## Layout

| Crate | Role |
|---|---|
| `loker-core` | Domain model, eligibility verdicts, country mapping, SQLite store (no transport code) |
| `loker-sources` | Source adapters (Himalayas so far) |
| `loker` | The `loker` binary (CLI; later also the MCP and REST server) |

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
