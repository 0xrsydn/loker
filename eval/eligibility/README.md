# Eligibility eval

Measures the verdict loker gives a viewer (yes / maybe / unknown / no)
against verdicts labelled by Opus and DeepSeek from the whole posting. Run
it by hand when changing location parsing, an adapter's location mapping,
the merge or the verdict; it isn't part of CI. A wrong yes sends someone to
a job they can't take, so yes precision comes first; a wrong no turns away
someone who could apply.

```sh
cargo build --release -p loker-sources --example eligibility
cd eval/eligibility
python3 eligibility_eval.py parity [--db PATH]   # rebuilt vs stored; equal unless the rules changed
python3 eligibility_eval.py sample               # once: sample.json (tuning), holdout.json
JUDGE=opus python3 eligibility_eval.py label [--set holdout]   # cached
JUDGE=deepseek41 python3 eligibility_eval.py label [--set holdout]   # DeepSeek v4.1 via Pi (opencode-go), high reasoning
python3 eligibility_eval.py agree [--set holdout]
python3 eligibility_eval.py score [--set holdout] [--bin PATH] [-v]
```

## How it works

- **Current code, stored data.** `crates/loker-sources/examples/eligibility.rs`
  re-reads every listing's stored raw item through the adapter
  (`job_from_raw`, the function ingest uses), merges the listings as the
  store does (`dedup::resolve`) and gives the verdict per viewer. A rule
  change is measured without fetching again. `parity` checks the pipeline:
  on rules unchanged since a snapshot, every rebuilt eligibility equals the
  stored one (it did, for all 10,691 jobs in 5 snapshots, before the
  changes below). An employer board's `remote` flag isn't stored; it is
  recovered when the stored scope was known and re-reading without the
  flag gives none (226 Binance jobs).
- **Viewers.** US (UTC-5), DE (+1), BR (-3), IN (+5.5), ID (+7): each wants
  to work remotely from home and won't relocate or commute.
- **Labels.** Prompt v1 in the script. The labeler sees the board's own
  fields (long text dropped), a line on what each board's fields mean as
  the board documents them, and the description; never loker's reading.
  When a board field and the description disagree, the more specific one
  wins.

## Sets

| set | from | jobs |
|---|---|---|
| `sample.json` (tuning) | `eval/pay/snapshot.db` (2026-11-19), 28 per source | 196 |
| `holdout2.json` | `snapshot-h2.db` (aggregators and 21 new employer boards, fetched 2026-11-20 14:30, after the description reader), `snapshot-h2-himalayas.db` (Himalayas pages 1-50) and the jobs of `snapshot-agg.db` no set drew; every job any other snapshot has, by id or URL, left out | 105 |
| `holdout.json` | `snapshot-agg.db` (aggregators fetched 2026-11-20, URLs not in the tuning snapshot) and the 36 employer boards of `eval/visa` (`snapshot-boards*.db`), up to 20 per source, 3 per company | 123 |

The tuning snapshot's employer boards are 4 companies (Binance, Palantir,
Clera, Mercor); the holdout's are 25. Remote OK had only 8 jobs not
already in the tuning snapshot. A first holdout draw from
`eval/role/snapshot-fresh.db` had no Jobicy, Remote OK or We Work Remotely
jobs; it was discarded before any label or text was read.

Labelers agree on 926/980 tuning verdicts and 585/615 holdout verdicts;
most differences are maybe against yes or no.

## Results (2026-11-20)

Each row is jobs x 5 viewers. "Wrong yes" is a yes the label calls no
(a yes the label calls maybe is counted in precision, not here).

| rules | set | yes right | wrong yes | no right | wrong no |
|---|---|---|---|---|---|
| before (5f4f0b6), Opus / DeepSeek | tuning | 169/256 / 186/256 | 71 / 67 | 586/594 / 594/594 | 0 / 0 |
| now, Opus / DeepSeek | tuning | 159/175 / 161/175 | 9 / 13 | 612/620 / 620/620 | 0 / 0 |
| before (5f4f0b6), Opus / DeepSeek | holdout | 54/148 / 62/148 | 78 / 82 | 371/382 / 382/382 | 3 / 0 |
| WWR and merge fixes (e66140d), scored once | holdout | 37/46 / 37/46 | 3 / 5 | 388/399 / 399/399 | 3 / 0 |
| + places and description, tuning | tuning | 174/183 / 176/183 | 0 / 4 | 701/709 / 709/709 | 0 / 0 |
| **+ places and description, scored once** | holdout | **44/53 / 44/53** | **3 / 5** | **469/480 / 480/480** | 3 / 0 |
| before places and description (74b2364), Opus / DeepSeek | holdout2 | 40/43 / 36/43 | 2 / 3 | 334/346 / 340/346 | 2 / 6 |
| **now (bde0ee2), scored once**, Opus / DeepSeek | holdout2 | **43/48 / 39/48** | **2 / 4** | **367/379 / 373/379** | **2 / 6** |

What changed, on the tuning set:
- **We Work Remotely's `country` field is read.** 76 snapshot items say
  "Anywhere in the World" as region and list countries in `country`
  ("🇨🇦 Canada and 🇺🇸 United States of America"); the adapter ignored it.
- **"Anywhere in the World" alone is unknown, not worldwide.** It is the
  board's default (246 of 530 snapshot items carry it without `country`),
  and in the sample it held for 2 (Opus) or 4 (DeepSeek) of 11 such jobs;
  the rest restrict in their text ("U.S.-based", "legally eligible to work
  in Bulgaria"). This costs true yeses: 54 → 37 on the holdout (Opus), so
  17 verdicts that were right now say unknown, for 75 fewer wrong yeses.
- **The merge prefers a listing that names places.** A job on Jobicy
  ("Canada") and We Work Remotely ("Anywhere") took the more confident
  "worldwide".

Also found by the property tests (`crates/loker-core/tests/verdict_props.rs`),
not by this eval: viewers at UTC+13 and +14 were told no for hours listed as
UTC-11 and -10.

Then, on the tuning set:
- **States, provinces and cities map to their country**
  (`country::place_country`): Remote OK writes "Dallas, Texas", "Warsaw",
  "Colorado". A state or province counts as the country; a city alone may
  be an office, so it is maybe inside the country and no outside it.
- **The description fills in what the board leaves open**
  (`eligibility_text`): "based in Colombia, Mexico, or Argentina", "remote
  within the U.S.", "eligible to work in X", "CET (+/- 3 hours)". It never
  overrides a place or hours the board stated, and skips conditions ("if you
  live in"), exclusions ("outside the US") and the company ("our team is
  based in"). A spot check of 40 changed jobs outside the sample found two
  misreadings ("is" as Iceland, an "if you live in" clause), both fixed.

On the holdout, scored once after both: unknown fell from 165 verdicts to
75 and settled verdicts found rose from 425/591 to 513/591 (Opus), with
no new wrong claims. The holdout has now seen this reader; a change to it
needs a fresh holdout.

Then, from unknowns across the whole tuning snapshot (not the labelled
sample, which they don't change): "located in and authorized to work in
the US", "residing in the following US states: ...", "any city within
Italy or Spain", "either the NYC or Boston metro areas", "hiring for this
role in Canada or the United States", "CET plus or minus three hours". All
23 jobs this changed in the tuning snapshot and in
`eval/role/snapshot-fresh.db` were read; that found and fixed "AZ, CA, CO"
read as Azerbaijan, Canada and Colombia (a list of states is the US),
"Europe, Singapore, or wherever you are" read as a restriction, and a later
city ("Based in New York, you will...") undoing a country. Reading them
means `snapshot-fresh.db` is no longer unseen for these phrasings. The
holdout, scored again as a check (its second use), is unchanged: 44/53 and
513/591.

What is left, in the tuning snapshot's 263 jobs still unknown with no place:
- 223 state nothing: no place, hours or requirement. Unknown is the answer.
- 10 state hours only; viewers outside them already get no.
- 11 have only pay by location ("for most US-based locations").
- 19 state a requirement loker can't map: sales territories ("Great Lakes
  region (IL, MI, OH)", "live local to the territory"), overlap percentages
  ("70-80% overlap with San Francisco"), "an EU-compatible time zone".
  Preferences ("US or Europe preferred") are left out on purpose.

`holdout2` (105 jobs, fetched after the reader was written, nothing in it
read) confirms the reader on unseen jobs: unknown verdicts 135 -> 85,
settled verdicts right 374/495 -> 410/495 (Opus) and 376/518 -> 412/518
(DeepSeek), with wrong claims flat (wrong yes 2 -> 2 by Opus, 3 -> 4 by
DeepSeek; wrong no unchanged). Labelers agree less here (474/525), mostly
maybe against yes; DeepSeek's 6 wrong noes are jobs it reads as open where
Opus doesn't. No Lever board was new, so Lever has no unseen test since
`holdout`. `holdout2` is now used; the next change needs a new one.

### Workable (a new source, 2026-11-20)

`workable.json` (60 jobs, 46 companies) was Workable's first test, before
any rule looked at it: yes right 39/42, but 33 (Opus) or 66 (DeepSeek)
wrong noes. A remote job's Workable place is the employer's, not a limit:
17 of 60 said "work from anywhere", "we hire worldwide", "based outside the
US" or named other places. The adapter now lets places the description
names win, gives no claim when the description suggests it is open wider
(`eligibility_text::open_wider`, used only to hold back a no), and keeps
Workable's place otherwise. On `workable.json`, now tuning: wrong noes
33 -> 5 / 66 -> 13.

`workable2.json` (60 jobs fetched after, none in `workable`), scored once:

| judge | yes right | wrong yes | no right | wrong no |
|---|---|---|---|---|
| Opus | 29/38 | 4 | 233/247 | 11 |
| DeepSeek | 29/38 | 6 | 222/247 | 21 |

Weaker than the other sources (holdout2: 43/48 and 367/379). The misses:
jobs the employer marked remote that aren't ("field-based ... travel to
patients' homes"), a "Location:" line in the description ("Location:
Remote – LATAM", "Location: Bangladesh, India, ..."), "open to candidates
worldwide" read but not used by the Workable rule (a bug), and "live
wherever you like".

Second round, on those: "Location:" lines in the shared reader (a line
naming a city is the office, unless it says remote), "open to candidates
worldwide" now used by the Workable rule, "wherever you like". The tuning
sample and both holdouts are unchanged by it. `workable3.json` (60 jobs
fetched after, none in `workable` or `workable2`), scored once with Opus;
DeepSeek ran out of credit (HTTP 402) after 20 jobs:

| judge | yes right | wrong yes | no right | wrong no | no, labelled maybe |
|---|---|---|---|---|---|
| Opus, 60 jobs | 29/36 | 2 | 222/249 | 6 | 21 |
| DeepSeek, 20 jobs | 15/15 | 0 | 72/80 | 8 | 0 |

DeepSeek v4.1 Flash (OpenCode Go, high reasoning; `deepseek41`, a new
labeller kept apart from the old one) then labelled all 60: yes right
31/36, wrong yes 3, no right 210/249, wrong no 37. It agrees with Opus on
249/300 verdicts, mostly answering yes where Opus says no (19) or maybe
(21): it reads Workable's remote jobs as open more widely. It is Workable, not the
labeller: on holdout2 (all sources) DeepSeek v4.1 agrees with Opus on
496/525, more than DeepSeek v4 Pro did (474/525), and where it differs it
is stricter (no where Opus says maybe, 21), not looser. Scored with it,
holdout2 gives yes right 43/48 and no right 381/384 (2 wrong noes), against
37 wrong noes on workable3.

| holdout2 scored by | yes right | wrong yes | no right | wrong no | agrees with Opus |
|---|---|---|---|---|---|
| Opus | 43/48 | 2 | 372/384 | 2 | |
| DeepSeek v4 Pro | 39/48 | 4 | 378/384 | 6 | 474/525 |
| DeepSeek v4.1 (high) | 43/48 | 3 | 381/384 | 2 | 496/525 |

So both labellers say the same about Workable: a no for viewers outside a
remote job's listed country is not reliable there (Opus: 6 wrong and 21
maybe; v4.1: 37 wrong), while the listed country's yes holds.

Third round: eligibility became `workplace`, places and `elsewhere` (see
ARCHITECTURE.md), and a Workable remote job's place is now `not_stated`
for everyone outside it (unknown), or `open` (maybe) when the posting
hints so ("work from anywhere", "based outside the US"); places the
description names stay `excluded`. Every other set scores as before.
`workable4.json` (60 jobs from BR, GB and US, none in the earlier sets),
scored once:

| workable4 | yes right | wrong yes | no right | wrong no | settled found |
|---|---|---|---|---|---|
| Opus, previous rule | 37/37 | 0 | 239/248 | 8 | 276/299 |
| Opus, now | 39/39 | 0 | 55/55 | 0 | 94/299 |
| DeepSeek v4.1, previous rule | 36/37 | 0 | 188/248 | 48 | 224/287 |
| DeepSeek v4.1, now | 38/39 | 0 | 55/55 | 0 | 93/287 |

No wrong yes or no under either labeller. The cost is silence: 193 verdicts
are now unknown, which the labellers split on (Opus: 184 no, 8 yes;
DeepSeek: 133 no, 48 yes); they disagree most on exactly these. An agent
filtering with `minVerdict` sees no difference (unknown and no are both left
out); one reading verdicts is no longer told no where the posting doesn't
say. Workable can be enabled for a viewer's own country: its yes held 39/39
and 38/39 here.

Wrong noes fell from 11 to 6 (Opus), wrong yeses from 4 to 2, but a no is
still right 89% of the time against 97% for the other sources, and
DeepSeek's 20 jobs alone give 8 wrong noes. Workable stays opt-in.

Citizenship and clearance are deferred, on purpose. Of the tuning
snapshot's 1,749 jobs, 178 state citizenship, permanent residency or a
clearance outside equal-opportunity text, and the location verdict already
tells all four non-US viewers no for 175 of them (and the US viewer no for
168: most are on-site). A citizenship field would change about 10 answers,
for people living in the US without citizenship. Worth building when
loker's users include them, or sources heavy in government or defense work
are added.
- Binance is ingested as an all-remote board; its postings say
  "remote-first ... may vary", which the labels read as maybe (3 tuning
  yeses).
- The 3 Opus wrong noes on the holdout are all Himalayas jobs, where the
  board states the countries; DeepSeek labels all three no.
- Small sets: a holdout yes precision of 44/53 has a Wilson 95% interval
  of 71-91%.

Jobs read while changing rules: every tuning-set miss listed by `score -v`.
The holdout's text and labels were not read before it was scored.
