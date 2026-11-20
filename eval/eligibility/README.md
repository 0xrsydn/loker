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
JUDGE=opus python3 eligibility_eval.py label [--set holdout]   # cached; JUDGE=deepseek needs DEEPSEEK_API_KEY
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

What is left:
- **75 unknown verdicts on the holdout** (50 no, 16 yes by Opus): phrasings
  the reader doesn't know yet ("exclusive to Filipino locals", hours as
  "overlap with US Eastern Time").
- Binance is ingested as an all-remote board; its postings say
  "remote-first ... may vary", which the labels read as maybe (3 tuning
  yeses).
- The 3 Opus wrong noes on the holdout are all Himalayas jobs, where the
  board states the countries; DeepSeek labels all three no.
- Small sets: a holdout yes precision of 44/53 has a Wilson 95% interval
  of 71-91%.

Jobs read while changing rules: every tuning-set miss listed by `score -v`.
The holdout's text and labels were not read before it was scored.
