# Architecture

How loker is put together, the rules every change follows, and when the design
should change. Read this before changing anything in `loker-core` or adding a
source. The README covers what loker does and how to run it.

## The pipeline

loker is an ETL pipeline with a read side. Ingest runs on a schedule; search
only ever reads the local database, never the sources.

```
Extract    adapter fetch            loker-sources/<source>.rs   network
Transform  adapter transform        loker-sources/<source>.rs   no IO: raw item -> NewJob
           normalize, country       loker-core                  no IO: shared rules
Decide     dedup::cluster/resolve   loker-core/dedup.rs         no IO: which listings are one job
Load       Store::ingest            loker-core/store.rs         database: load rows, ask, write
Read       api::search_jobs, ...    loker-core/api.rs           typed v1 contract
Interfaces CLI, MCP                 loker/src                   translate only
```

## Where code goes

- **Specific to one source?** It goes in that source's adapter (field names,
  date formats, a `workplaceType` that hints at remote).
- **Decides something?** It's a function with no IO, in `loker-core`, taking
  data in and returning an answer.
- **Touches the database?** It goes in `store.rs`, which loads what a rule needs
  and writes back what it decided.
- **Shows something to a user or agent?** The type lives in `loker_core::api`;
  the CLI and MCP only translate it.

## Rules

Each rule says why it exists and what enforces it.

**1. Listings are the record; jobs are derived.**
Each source's listing is stored in `source_jobs` with its raw payload and its
normalized `NewJob`. A canonical job is always rebuilt from its live listings.
*Why:* dedup doesn't depend on ingest order, and a better normalizer can be
applied to old data by rebuilding, without fetching again.
*Enforced by:* `tests/dedup.rs` (every ingest order gives the same jobs).

**2. Rules decide, the store carries out.**
Decisions are functions with no IO; the store loads rows, calls the rule, and
carries out the answer. *Why:* a rule can be read and tested without a
database. *Enforced by:* review. `dedup` follows it; liveness is the known
exception (see below).

**3. Adapters map fields; the core normalizes.**
An adapter turns its source's format into `NewJob` and passes source-specific
hints (Ashby's `workplaceType`, Himalayas' country list) to the shared rules in
`normalize`. *Why:* fixing a rule once fixes every source, and a new source is
mostly field mapping. *Enforced by:* `loker verify` and the e2e stub.

**4. One typed contract, checked three ways.**
CLI `--json` and MCP return the same `loker_core::api` types (contract v1).
`loker verify` checks adapters against live sources, the e2e stub checks them
against fixed sample data, and `contract_schemas_match_committed_v1` catches
contract drift. *Why:* every interface agrees, and an upstream change shows up
as a failing check instead of bad data.

**5. The stored job is a format, not just a struct.**
`NewJob` is saved as JSON, hashed, and read back on every rebuild, and SQL reads
some of its fields by name. New fields are optional and omitted when empty
(`#[serde(default, skip_serializing_if = "Option::is_none")]`, as `expires_at`
is), and nothing is renamed without a migration. *Why:* otherwise every stored
listing reads as changed at once, or old rows stop parsing and fail the whole
ingest. *Enforced by:* `tests/storage.rs` (every JSON path the SQL reads still
resolves).

**6. Derived values carry their source and confidence.**
Eligibility records its method, confidence and evidence. *Why:* when sources
disagree, the merge has a principled winner, and anyone can see why a job got
its answer.

## Trade-offs

The design chooses one kind of error over another. Each choice is deliberate.

| Prefer | Over | What you see instead |
|---|---|---|
| A missed merge | A wrong merge | When a source lists the same company and title twice (one per city), every listing under that key stays separate. A duplicate may show; two different jobs never become one. |
| A job wrongly left open | A job wrongly closed | A fetch that would close more than half of a source's live listings, or returns nothing, closes nothing (`held`). The 30-day staleness rule cleans up later. |
| Lenient parsing plus monitoring | Strict source schemas | Adapters tolerate messy upstream data; `loker verify` fails when a source's parse rate or required fields degrade. |

## How it works

**Dedup.** A listing's match key is (`company_key`, `title_key`), computed by
the store. Under one key, live listings merge when each source has at most one;
if any source has two, all stay separate. A merge keeps the oldest canonical
job's id. The merged job takes identity and text from employer boards first,
then by source id; eligibility from the most confident source; pay from a
source with a USD figure.

**Liveness.** A listing goes `gone` when a complete fetch of its source no
longer has it (`absent`), its source's expiry passes (`expired`), or nobody has
seen it for 30 days (`stale`), and goes `live` again when seen. A job is active
exactly while one of its listings is live.

**Ids.** A canonical id is `j_` plus a hash of the first listing's source and
id, assigned once and never recomputed.

## Changing things

**A new source**
1. A module in `loker-sources` with `fetch` and `transform`, and an arm in each
   `Source` method.
2. Use the shared normalizers; keep source quirks in the adapter.
3. A route with one sample payload in the e2e stub.
4. `loker verify <source>` passes against the live source.

A company board on an existing platform (`greenhouse:<board>`) is a config line,
not code.

**A new field on a job**
1. Optional on `NewJob`, omitted when empty (rule 5).
2. Each adapter fills it or sets `None`; the compiler lists every one.
3. A precedence rule in `dedup::merge`.
4. Storage: today a `canonical_jobs` column and a migration, plus an entry in
   the hand-listed hash in `rebuild_canonical`. Read the triggers below first.
5. The `Job` output type, then regenerate `schema/v1` (an optional field is an
   additive change within v1).

**A new rule**
A function with no IO next to the rules it belongs with, called from the store.
If it needs a test, test the function directly with a table of cases (see
`cluster_rules` in `tests/dedup.rs`).

## Known exceptions

- **Liveness decisions live inside `Store::ingest`:** the truncation guard,
  expired-or-stale, reopening. They should move into a pure module like `dedup`.
- **`rebuild_canonical` lists the hashed fields by hand.** A field left off the
  list can change without the job being rewritten.
- **General cleanups are repeated per adapter, and have drifted:** Himalayas
  doesn't run `clean_url` on its URL, only Jobicy decodes HTML entities in
  titles, and Himalayas converts HTML without the shared `html_to_md`.
- **Matching is an exact key.** Near-miss titles ("Sr. Backend Eng" vs
  "Senior Backend Engineer") don't merge.

## When to change the design

Nothing here is needed now; each change waits for its trigger.

| Change | Trigger |
|---|---|
| Store the merged job as JSON in `canonical_jobs`, with generated columns only for filtered fields, and hash that JSON | Before the first new job field (YOE, role) |
| A shared cleanup step after `transform` in `fetch_batch` (trim, decode entities, `clean_url`, drop empty descriptions) | Before the next adapter |
| An `Adapter` trait and shared fetch methods (JSON-LD, RSS) | About 10 adapters, or the first HTML source |
| Concurrent `ingest --all` with per-host rate limits | Hundreds of company boards |
| More signals inside `dedup::cluster` (location, apply-URL domain) to tell apart listings that share a key | `sources eval` shows blocked keys hiding real separate jobs |
| Fuzzy matching across keys (a new candidate query, e.g. by company) | `sources eval` or real use shows missed merges. A real design change, not a tweak. |

New kinds of data (company profiles, salary data, application tracking) get
their own types and follow the same pipeline; they don't go into `NewJob`.
