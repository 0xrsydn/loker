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
`normalize`. Cleanups every job needs (HTML entities and whitespace in titles,
`clean_url` on every URL, empty text to `None`) run once, in `normalize::tidy`,
which `fetch_batch` applies after every `transform`; adapters don't repeat
them. *Why:* fixing a rule once fixes every source, and a new source is
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
ingest. A renamed field doesn't fail in SQL either: its JSON path just reads
NULL. *Enforced by:* `tests/storage.rs` (pins every JSON path the SQL reads
through a number it feeds).

**6. Derived values carry their source and confidence.**
Eligibility records its method, confidence and evidence. *Why:* when sources
disagree, the merge has a principled winner, and anyone can see why a job got
its answer.

Eligibility is three answers, so gray information has a value instead of
a guess: `workplace` (remote, hybrid, onsite, unknown), the places
(countries, regions; an office job's office) and `elsewhere` (excluded,
not_stated, open: what the posting says about everyone outside its
places). The verdict is a small table over them: inside the places, remote
is yes and unknown workplace is maybe; outside, excluded is no, not stated
is unknown and open is maybe; office jobs are no unless the viewer accepts
that office. The verdict is computed per request from the stored
eligibility and the viewer, never stored. `scope` is derived from the three
and still served, for earlier clients. *Why:* a list of places is not always a limit ("USA Only"
is; an employer's office on a remote Workable job isn't), and both
labellers found the no it implied wrong there.

Eligibility comes from the board's fields first. The description
(`eligibility_text`) only fills in what they leave open: places when the
board names none ("based in the US" under "Anywhere in the World"), hours
when it gives none, never for on-site or hybrid jobs. It runs at ingest, in
`job_from_raw`, since search filters on the verdict. A job whose workplace is
unknown counts as remote only on a board that lists nothing but remote jobs
(`Source::remote_only`; not Y Combinator, which lists on-site jobs too) or
an employer board configured `remote = true`.

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
then by source id; eligibility from a source that names places if any does
(the narrower statement), then the most confident; pay from the first source
that states base pay; visa and experience from the first that states them.
Categories are the union of every listing's, since each is evidence of what
the job is.

**Storage.** A canonical job is a row holding its merged job as JSON
(`canonical_jobs.job`, a `NewJob`), hashed as a whole together with its
status, so an unchanged rebuild writes nothing. SQL reads only generated
columns (`canonical_url`, `remote_scope`, `posted_at`, `title`, `company`) and
JSON paths (`tests/storage.rs` pins each one).

**Search.** Any word of the query matches. Jobs with every word rank first,
then by bm25 with a title match weighted five times: `eval/search` found
all-words-first as good as score alone, and it never comes back empty when
some word matches; a title weight of 5 beats 1, and 10 is no better. The
verdict, level, role and years filters are derived when a job is read, so
they run in Rust over every match: the only way to fill a page and count
`total`, and a few thousand rows take milliseconds. The SQL pre-filter on
the viewer's country must never be stricter than the verdict.

**Fetching.** `ingest --all` and `verify` fetch concurrently, limited per API
host (`Source::host`): `FETCHES_PER_HOST` in flight, starts `HOST_START_GAP`
apart. Every Greenhouse board is one host, every Ashby board another. A 429 or
503 pauses the whole host (`COOLDOWNS`, keyed by URL host, for `Retry-After`
or an exponential backoff, capped at `MAX_COOLDOWN`), since the limit is on
us, not on the one request; the request is asked again, up to four times. Writes
stay on one task, in the order fetches finish; that is safe because dedup
depends only on what is stored, never on ingest order. Pauses between pages
of one source (Himalayas, Workable, YC) stay in the adapter.

**Boards.** `loker discover` collects candidate boards from a finder (Common
Crawl, or a file of URLs or ids), turns each into a `Board` (`board.rs`, no IO), and keeps one
row per `Board::key` in `boards`, with every finder that named it in
`board_finds`. A candidate becomes `live`, `empty` or `missing` only by asking
its ATS's API (`discover::check_board`, paced like ingest), so a finder can be
noisy without being wrong: it decides coverage, never correctness. A check
that gets no answer leaves the board `unchecked` with its error, to be asked
again. A board is checked again once its last check is `RECHECK_AFTER` (7
days) old, so an `empty` board that starts hiring turns `live` and a
`missing` one that comes back is found; `Store::boards_due` puts unchecked
boards first, then the oldest checks, and `--check` caps the run. Fetching a
board in ingest is a check too (`Store::record_board_fetch`): jobs keep it
`live`, none make it `empty`, a 404 makes it `missing`, other errors change
nothing. So a board ingested daily is never asked twice, and one that dies
fails one ingest, then is no longer fetched. The check also stores the ATS's own spelling of the slug (Ashby's
`jobUrl`; Greenhouse lower case) and, for a live board, the company as the
ATS names it (`board::Company`): Greenhouse's `company_name` from the same
answer; for Ashby and Lever, whose APIs carry no company, one more request to
the board's own page (the organization in Ashby's `window.__appData`, with its
website; Lever's `<title>`). A failed page leaves the name empty, not the
check failed. `ingest --all` passes the name as the board's `company`, as a
configured `company` would. A board with jobs whose name says it isn't for
the public (`Company::names_a_private_board`: internal, referral-only,
alumni, talent-community, LinkedIn or third-party mirror) is `private`, and
never ingested: its jobs are closed to the reader, or copies of a main
board's under another name. The phrases are whole words, so "HiddenLayer"
and "SandboxAQ" stay live; on the 2026-11-23 check they marked 28 boards,
all correctly (eval/discovery). The name is cleaned before both uses
(`Company::new`): entities decoded, careers-page wording dropped ("Kaseya
Careers", "Careers at Tide", "LP Careers Page"), stray edge punctuation
trimmed; a name that would become empty is kept as it was.
With `[discovery] ingest = true`, `ingest --all` adds every `live` board whose
key config.toml doesn't list, after the configured sources.

**Liveness.** A listing goes `gone` when a complete fetch of its source no
longer has it (`absent`), its source's expiry passes (`expired`), or nobody has
seen it for 30 days (`stale`), and goes `live` again when seen. A job is active
exactly while one of its listings is live. A complete fetch's ids come from
its raw items, so an item that fails to transform still counts as present;
if any item has no id, the fetch closes nothing.

**Seniority.** Derived when a job is read, like a verdict, so every stored
job has it without a re-fetch: level words in the title first, else the level
the source states (`normalize::seniority`). About 40% of jobs name a level.
Jobicy's `jobLevel` "Director" is ignored: it is set on account executives
and BDRs as often as on directors.

**Pay.** Optional, and only what the posting states, in its own currency:
`base` ranges (salary or wage, one per level or location when there are
several), `extras` (equity, bonus, commission, OTE, sign-on; figures when
stated), and `usdMonthMin/Max` for comparing, from USD amounts only.
`normalize::money` drops a figure rather than keep it wrong: no currency
code, no period for base pay, `min > max`, a top over 10× the bottom, or a
dollar-like currency (USD, EUR, GBP, ...) outside 100..=100k a month.
A zero end counts as not stated, since sources write 0 for none. A `$` is
another dollar when a place or code names it ("Ontario based" is CAD); when
two are named, the figure is dropped.
"Competitive" is no pay; "competitive salary + equity" is pay with only an
extra. A source with no pay fields gets pay from its description in `tidy`
(`pay_text.rs`, `method: text`): a figure counts only with a currency, pay
words around it and a period, and never next to benefit, funding or sales
words. A salary with no period is a year only at 15,000 or more and only in
currencies whose salaries are yearly at that size (USD, EUR, GBP, CAD, AUD,
NZD, CHF, SGD); 25,000 ZAR may be a month. Extras count only within three
lines of base pay or where the posting offers pay ("eligible for a bonus"),
never alone. `eval/pay` measures it.

**Visa.** `visa.sponsorship` is `offered`, `limited` (case by case,
transfers only, some locations) or `not_offered`, with the sentence it was
read from; absent when the posting says nothing about sponsorship. "Must be
authorized to work in the US" alone, a citizenship or clearance requirement
and equal opportunity text say nothing about it, nor does text that only
mentions sponsorship (an interview question, an HR duty): a wrong
`not_offered` turns someone away from a job they could take. A source that states it in a
field of its own (`NewJob::visa`, Y Combinator's three fixed values) fills it
at ingest and wins (YC's "US citizenship/visa not required" says nothing
about sponsorship); otherwise `visa_text.rs` reads it from the description
when a job is read (like seniority), so a rule change needs no re-fetch.
`eval/visa` measures the reading.

**Experience.** `experience.minYears` is the years a job requires (0 when
it says none is needed), with `maxYears` for a range and the phrase it was
read from; absent when no number is stated ("several years" is none). It is
separate from seniority: a title's level and the years asked don't line up
across companies. Y Combinator's own field (`minExperience`) fills it at
ingest and wins; otherwise `experience_text.rs` reads the description when
a job is read. Only required years count: a preferred heading or "5+ years
preferred" doesn't; of several required numbers the largest is the bar, of
alternatives ("5 years with a bachelor's or 3 with a master's") or levels
("For Senior: 6+") the smallest. A number too high would hide a job from
someone who meets the real bar, so unclear phrasing and a company's own age
("drawing on 30 years of experience") are left out, and the
`experienceYears` filter keeps every job that states no years. `eval/experience`
measures it.

**Roles.** `role.rs` is the vocabulary for what a job does: families
(`engineering`, `ai`, `data`, `product`, `design`, `go_to_market`, `support`,
`operations`, ...) with roles in them (`backend`, `ai_engineering`,
`product_management`, ...), and modifiers that cut across them
(`forward_deployed`, `founding`). Each role has one home family and may also
sit in others (`data_engineering` is in data and engineering); `ai` sits
inside engineering. Filters name any id or modifier and `Role::within`
resolves them, like regions. Role is function only: seniority, domain (game,
web3), company stage and employment type are other fields, and specialisms
(computer vision, NLP) are left to search. A job stores its
most specific id, a family alone when the evidence says no more, or nothing:
there is no "other" role. Adapters keep the source's category, department
or team as written (`NewJob::category`); only `classify` decides which
phrases name a role. A job has at most three roles, a primary and two
secondary, as LinkedIn caps job functions. `JobRole` records method,
confidence, evidence and the vocabulary version (rule 6).

`classify.rs` decides a job's role from the merged job: phrases in the title
first (multi-word role nouns before single keywords, a matched phrase used
up), then the source's category when the title names no specific role, then
a role named early in the description. The team a manager runs is a second
role ("Engineering Manager, Applied AI"); a domain word is not ("Data
Engineer, Marketing Analytics" is data engineering only). A category never
makes a role broader than the title says, and an engineer title takes only
an engineering category. `product_engineer` is not inside `fullstack` in the
vocabulary; the classifier gives product engineers `fullstack` as a
secondary role, which is how a fullstack filter finds them. It is stored with the job
(`canonical_jobs.role`) along with `classify::STAMP`; when the rules or the
vocabulary change, `Store::open` classifies stored jobs again, without a
re-fetch. `eval/role` measures it against labelled jobs.

**Providers.** Third-party services are optional and never on the search
path. `[providers.jev]` in config.toml (key from `OPENROUTER_API_KEY`) adds
Jev's choice over the vocabulary: `loker roles enrich`, or the end of
`ingest --all` (after every source is stored; a provider failure lands in
the report's `roles.errors` and doesn't fail the run), asks about active jobs
with no cached answer and stores each
in `role_answers`, keyed by vocabulary, question and what the model saw, so
an unchanged job is never asked twice. The model is a dated id
(`typesafe/jev-1.14-20261014`), since an alias moves between releases. The key leaves out the model id, so a
new model keeps the cached answers until a job changes; bump
`classify::QUESTION` to ask every job again. Storing answers classifies every
active job again rather than tracking which changed: a few thousand take
milliseconds. `classify::classify_with` prefers the
model unless a title names the role and the model is under 0.6; modifiers
stay with the rules. Without the provider, roles come from the rules alone.
Jev's Decisions endpoint is alpha, so only `jev::ask` calls it; its
confidence is the probability of the chosen role, the number `eval/role` set
the 0.6 threshold on. The cache keys on the model config names, not the one
the response reports.

**Sources.** What each source's data means, as found when writing its
adapter. All of them share one HTTP client: User-Agent `loker/<version>` and
nothing personal, a 30 s timeout, and the platform's TLS roots plus Mozilla's
bundled ones, so the binary works without system CA certificates (minimal
containers, the Nix sandbox). A network failure (timeout, refused or dropped
connection, a cut-off body) is retried twice, after 2 s and 4 s; an HTTP
status never is (a 404 for a missing board is an answer), except Y
Combinator's 429.

- *Ashby:* unlisted postings (`isListed: false`) are reachable by link only,
  so they are skipped.
- *Greenhouse:* the canonical URL is built from board and id, because
  `absolute_url` varies (an EU host, a careers page with `?gh_jid=`). The
  description arrives as escaped HTML.
- *Himalayas:* remote only. With a country, the page-numbered search (which
  returns worldwide jobs plus those listing it); otherwise the cursor feed.
  An empty country list is worldwide, a list is the limit;
  `timezoneRestrictions` covering every zone is no restriction.
- *Jobicy:* remote only, so `jobGeo` is where the job is open.
- *Lever:* pages of 25 until a short page, so the fetch is the complete
  board. One unpaged request also returns everything, but Lever sends each
  posting's HTML (about 20 KB) uncompressed at as little as 15–60 KB/s, so a
  large board outlasts the timeout; each page gets two minutes. Lever's ISO
  `country` fills in an on-site or hybrid job's country. For a remote job it
  is the posting location's country, used (`Source::posting_country`, after
  the description is read) only when the location names a place no rule
  reads, like Veeva's "Massachusetts - Boston": on the 2026-11-23 sample it
  filled 685 of 6,742 jobs, and the eligibility holdout gained 5 right
  verdicts and no wrong ones. A location of just "Remote" may carry the
  employer's own country, so it stays unknown, as does a lone city (the
  normalizer's rule).
- *Remote OK:* the first array element is a legal notice. Salaries are USD a
  year, 0 means not given, and the pay bounds drop junk values.
- *We Work Remotely:* RSS. The all-jobs feed holds only about ten of each
  category's newest, so each category feed is read too and merged by guid;
  a failing category feed is skipped (its slug may have changed), the
  all-jobs feed must load. Not a complete list: expiry and staleness close
  its jobs. `region` is who may apply; `country`, when present, is narrower
  and wins. "Anywhere in the World" alone is the board's default, not
  evidence (2 of 11 such postings in `eval/eligibility` were open to all five
  viewers), so it is remote with `elsewhere` not stated.
- *Workable:* the search jobs.workable.com uses, not a published API, so it
  may change without notice (`loker verify` notices). Remote jobs only, 20 a
  page, a second apart. A remote job's location is the employer's, not a
  limit (17 of 60 in `eval/eligibility` said "work from anywhere" or named
  other places), so places the description names win; otherwise Workable's
  place is a yes for the people in it, and everyone else gets `open` when the
  posting hints so, else `not_stated`. The stored description adds the
  requirements section, which often says where the hire must be. Only
  staleness closes its jobs.
- *Y Combinator:* no JSON API; each page is Rails + Inertia with its data in
  one `data-page` attribute. Listing routes are capped (20 on the landing
  page, 36–50 per role or location) and ignore pagination, but the sitemap
  is complete and each company page lists all its jobs: one sitemap request,
  then one per company. Company pages carry no descriptions, so pay, visa
  and experience come from the board's fields. The apply link is a login
  redirect, so the posting is the apply URL. Only an age is given ("23
  days", "over 1 year"), so `posted_at` is that age back from the first
  fetch that saw the listing; the store keeps that date afterwards
  (`SourceMeta::dated_by_age`), or every fetch would move it and rewrite
  every listing. The site
  rate-limits per path with minutes-long 429s and no `Retry-After`: pages
  are a second apart and a 429 is retried after 20 s and 40 s. About 10% of
  the sitemap's job URLs already 404. A job refreshes only when its company
  page is fetched and staleness closes it after 30 days, so `--pages 50` (40
  companies a page) covers the board; smaller runs take the next window each
  day of the shuffled, undated sitemap. A location is options separated by
  " / ": in "City, ST, CC" only the last part is a country (CA there is
  California); "Remote (US; CA)" is open to those countries only; "Remote
  (San Francisco, CA, US)" means living nearby, a maybe in that country; a
  bare "Remote" keeps the office countries and says nothing about anyone
  else; with no remote option the job is on-site at its first office.
- **Common Crawl** (a board finder, not a job source; `commoncrawl.rs`):
  `cluster.idx` (about 100 MB) is streamed and only the blocks whose key
  range can hold one of the ATS hosts are range-read from
  `data.commoncrawl.org`; `index.commoncrawl.org`'s query server returned
  504s throughout when measured. A 5xx on a block is retried like a network
  failure. A block's sort key (SURT) is lower-cased, so the board comes from
  each record's `url`, which keeps Lever's case. A crawl is read once
  (`discovery_runs`).

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
4. Storage: nothing. The merged job is stored and hashed as JSON; add a
   generated column (a migration) only if SQL must filter or sort on it.
5. The `Job` output type, then regenerate `schema/v1` (an optional field is an
   additive change within v1).

**A change to the role vocabulary**
Ids are stable: once data uses one it is never renamed, reused or given a new
meaning; labels and definitions may change, and clients must accept ids they
don't know. Bump `role::VERSION` and note the change here.
- Add: a `Role` variant, its `info()`, its place in `Role::ALL`.
- Split: new narrower roles with the old id in their `also_in`, so filters on
  the old id still find them.
- Move: change `home` and `also_in`; the id stays.
- Retire: set `replaced_by`; stop emitting it.
`tests/role.rs` checks the invariants (every id listed, homes are families,
no cycles, no name shared with a modifier).

**A change to the role rules**
Phrases in the `classify.rs` tables are lowercase whole words with
punctuation as spaces ("Full-Stack" is "full stack", "ML/AI" is "ml ai"),
most specific role first; the first match is the primary role. Bump
`classify::RULES`; `Store::open` then classifies stored jobs again.

**A new rule**
A function with no IO next to the rules it belongs with, called from the store.
Shared normalizers stay simple: add a rule when `loker sources eval` or an
eval under `eval/` shows what the current ones miss.
If it needs a test, test the function directly with a table of cases (see
`cluster_rules` in `tests/dedup.rs`).

## Known exceptions

- **Liveness decisions live inside `Store::ingest`:** the truncation guard,
  expired-or-stale, reopening. They should move into a pure module like `dedup`.
- **Matching is an exact key.** Near-miss titles ("Sr. Backend Eng" vs
  "Senior Backend Engineer") don't merge.

## When to change the design

Nothing here is needed now; each change waits for its trigger.

| Change | Trigger |
|---|---|
| An `Adapter` trait and shared fetch methods (JSON-LD, RSS) | About 10 adapters, or the first HTML source |
| A read pool beside the one writer connection `Store::open` opens | A long-running server with concurrent clients (REST) |
| More signals inside `dedup::cluster` (location, apply-URL domain) to tell apart listings that share a key | `sources eval` shows blocked keys hiding real separate jobs |
| Fuzzy matching across keys (a new candidate query, e.g. by company) | `sources eval` or real use shows missed merges. A real design change, not a tweak. |

New kinds of data (company profiles, salary data, application tracking) get
their own types and follow the same pipeline; they don't go into `NewJob`.
