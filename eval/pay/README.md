# Pay eval

Measures pay read from job descriptions (`crates/loker-core/src/pay_text.rs`)
against pay labelled by a model. Run it by hand when changing those rules;
it isn't part of CI. A wrong figure is worse than none, so precision comes
first.

```sh
cargo build --release -p loker-core --example pay_text
cd eval/pay
python3 pay_eval.py sample 300                  # once: tuning set -> sample.json
python3 pay_eval.py sample 150 --set holdout4   # a fresh holdout (unseen jobs only) -> holdout4.json
JUDGE=opus python3 pay_eval.py label            # label unlabelled jobs (cached); --set NAME for another set
JUDGE=deepseek python3 pay_eval.py label        # second labeler (needs DEEPSEEK_API_KEY)
python3 pay_eval.py agree                       # how far the two labelers agree
python3 pay_eval.py score [-v]                  # rules vs the Opus labels; --set holdout2 for the holdout
JUDGE=deepseek python3 pay_eval.py score        # vs DeepSeek's labels
python3 pay_eval.py score --bin /path/to/old/pay_text
```

## How it works

1. **Snapshot** (`snapshot.db`, local only): the role eval's snapshot
   (2026-11-19, 1,749 active jobs).
2. **Sets**: two thirds from jobs whose description has a currency next
   to a figure, one third from the rest, so misses outside that net show
   too. `sample.json` (300) is for tuning. A holdout (`holdout2.json`, 150)
   is drawn from jobs in no other set and not in `seen.json`, scored once
   per round of changes, and never tuned on. Once a round reads a holdout's
   failures, the next round draws a new one (`sample 150 --set holdout3`).
   `seen.json` lists every job whose text was read while writing the
   rules; add to it whenever you read one.
3. **Labels** (`labels-opus.json`, `labels-deepseek.json`): each labeler
   reads the whole description (pay is usually at the end) and answers base
   ranges (currency, period, min, max, label) and extras in loker's `Pay`
   shape, with the sentence it read them from.
4. **Score**: `examples/pay_text.rs` runs the rules on each description.

## Metrics

| Metric | Meaning |
|---|---|
| base ranges precision | Our ranges that match a labelled one: same currency and period, each end within 1% |
| base ranges recall | Labelled ranges we found |
| jobs exact | Jobs with pay where our ranges are exactly the labelled ones |
| with a wrong figure | Jobs we gave a range the label doesn't have: the number to keep near zero |
| extra kinds | Equity, bonus, commission, OTE, sign-on by kind |

## Results log

### Snapshot 2026-11-19, prompt v1, rules v1 (`cd8d64a`)

| | jobs | base precision | base recall | jobs exact | wrong figure | extras P / R |
|---|---|---|---|---|---|---|
| first draft, tuning sample | 300 | 92% | 79% | 81% | 10 | 97% / 48% |
| shipped, tuning sample (Opus labels) | 300 | 100% | 97% | 94% | 0 | 95% / 68% |
| shipped, tuning sample (DeepSeek labels) | 300 | 98% | 92% | 90% | 3 | 95% / 67% |
| shipped, **`holdout2`** (Opus labels) | 150 | **97%** | **82%** | 88% | 2 | 90% / 71% |
| shipped, **`holdout2`** (DeepSeek labels) | 150 | **97%** | **81%** | 87% | 2 | 92% / 66% |

`holdout2` is the honest number: drawn after the rules were written, from
jobs never read, scored once. The tuning sample is what the rules were
fitted to (and they were fitted to Opus's labels, hence the gap to
DeepSeek's). Labelers agree on base pay for 97% of the tuning jobs and
99% of `holdout2`. One `holdout2` posting lists 10 state-by-state hourly
ranges and is 10 of its 16 missed ranges.

`holdout.json`, the first holdout, is not clean: 12 of its 150 jobs had
pay lines read while the rules were written, before it was drawn, and it
was scored twice around one rule change. It gave 97% / 87% (97% / 88%
without the 12); kept for the record, not as a result.

Checks on the scorer: the labels scored against themselves give 255/255;
no prediction scores 0; moving each top 5% or swapping year and month
fails every range it touches. Extras are scored by kind only, not by their
figures. The sets over-sample money mentions, so "all" is a sample figure,
not a rate over all jobs.

Across the snapshot, replaying every live listing through `transform` and
`tidy` as ingest does, jobs with base pay go from 359 (source fields) to
924 of 1,749 (53%): mostly Lever and We Work Remotely, whose sources have
no pay fields. Greenhouse descriptions rarely carry pay; its pay is in a
separate field the board API returns per job.

Failures that round 2 worked on (`holdout2` is burned since):
- Wrong figures: "$" alone reads as USD in Canadian postings; "$1500 USD
  annually for professional development" read as base; an OTE stated
  four lines above its figure read as base.
- Missed: state-by-state "Min: $18.53, Max: $24.35" lists; "Rate:" and
  "Monthly Rate:"; "₱" and MYR; "100k€"; "$60,000,00"; "$185K–$215K base";
  two ranges per line; figures with no period (left out by design).

### Rules v2 (`720571a`)

Round 2 fixed those from the burned sets only, then drew `holdout3` (150
jobs, in no other set and not in `seen.json`), labelled it with both
models (148/150 agree on base pay) and scored it once.

| | jobs | base precision | base recall | jobs exact | wrong figure | extras P / R |
|---|---|---|---|---|---|---|
| tuning sample (Opus) | 300 | 100% | 96% | 94% | 0 | |
| `holdout2`, now burned (Opus) | 150 | 99% | 86% | 92% | 1 | |
| **`holdout3`** (Opus) | 150 | **99%** (82/83) | **94%** | 92% | 1 | 92% / 69% |
| **`holdout3`** (DeepSeek) | 150 | **99%** (82/83) | **93%** | 92% | 1 | 92% / 65% |

**Headline: novel formats.** Templates repeat across a company's postings
(Palantir's "The estimated salary range for this position is estimated to
be $X - $Y/year" is in 232 snapshot jobs), so a holdout job sharing pay
wording with a tuning job measures a format already tuned on. `score`
reports a `novel` group: holdout jobs whose pay wording (the 30 characters
before each figure, digits masked) is in no tuning job.

| novel formats | jobs (with pay) | base precision | base recall (Opus / DeepSeek) | wrong figure |
|---|---|---|---|---|
| rules v1 on `holdout2` | 86 (29) | 96% (27/28) | 67% / 63% | 1 |
| **rules v2 on `holdout3`** | 88 (35) | **97%** (35/36) | **87% / 85%** | 1 |

So: precision about 97% holds on unseen formats; recall on them is mid
80s, not the 94% of the whole holdout. The samples are small (36 ranges).

Base pay across the snapshot, replayed through `transform` and `tidy`:
359 (source fields) → 928 of 1,749 jobs.

Failures to work on next round (then draw `holdout4`):
- Wrong: "$300 certification bonus once you complete 200 productive
  hours" read as an hourly base rate ("bonus" after the figure should make
  it an extra; "hours" there is not a period).
- Missed: thousands separated by spaces ("1 520 000 - 1 900 000 CZK");
  "Total compensation: 12,000,000 - 23,000,000 HUF" (skipped as a total,
  though the labelers read it as base); "up to Php 88,000 Philippine
  Peso/month" (words between the figure and its period); "Pay Range
  $68,000—$70,000 USD" in one posting, to look into.

### Rules v2.1: one range stated twice

The hand-checked golden set (`eval/golden`) showed a posting stating
"$45,000 - $55,000" under "Compensation" and again under "Salary Range",
read as two ranges labelled "### Compensation" and "**Location:** Remote".
Now a pay heading is not a label, an unlabelled range that repeats a
labelled one is dropped, and ranges that all share figures are one. Across
the snapshot 34 jobs change and no figure does. The scorer matched ranges
many to one, so each copy of a duplicate counted as right; it now matches
one to one. Rescored (no new holdout; these sets were read before):

| | base precision | base recall | jobs exact | wrong figure |
|---|---|---|---|---|
| tuning sample, v2 rescored one to one | 97% (163/168) | 96% | 94% | 4 |
| tuning sample, v2.1 | 100% (163/163) | 96% | 95% | 0 |
| `holdout3`, v2 rescored one to one | 98% (81/83) | 94% | 92% | 2 |
| `holdout3`, v2.1 | 99% (81/82) | 94% | 94% | 1 |

So v2's logged precision (100% tuning, 99% `holdout3`) counted duplicates
as right; one to one it was 97% and 98%.

## What was wrong, and what is still unverified

Errors found by the audit, all corrected above:
1. `holdout.json` leaked: 12 of its jobs had pay lines printed while the
   rules were written (it was drawn later from the same pool), and it was
   scored twice around a rule change after its failures were read. Its
   97% / 87% was reported as the honest number; it was not.
2. Recall was overstated three times: 87% (leaked holdout), then 82% and
   94% on holdouts inflated by shared templates. On novel formats: 67%
   (v1), 87% (v2).
3. Coverage was computed from merged descriptions, not the ingest path:
   366 → 930 was reported; replayed through `transform` and `tidy` it was
   359 → 924 (v1).
4. Commit messages carry the wrong figures: 388a92a ("41 non-USD",
   "unchanged on all 321": 40 and 320) and cd8d64a (97% / 87%,
   366 → 930). They are not rewritten on main; this file supersedes them.
5. One `holdout2` job (`j_fe252f2d6e`) had its full JSON printed before
   any pay rule existed; it is out of the novel group.

Still unverified:
- Every label is from a model (Opus, checked against DeepSeek). Two
  models agreeing is not ground truth; no label has been checked by hand.
- Extras are scored by kind only; their figures are not checked.
- Every set is from one snapshot of one day.
- The sets over-sample jobs mentioning money; "all" is a sample figure.

