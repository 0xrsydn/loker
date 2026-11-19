# Pay eval

Measures pay read from job descriptions (`crates/loker-core/src/pay_text.rs`)
against pay labelled by a model. Run it by hand when changing those rules;
it isn't part of CI. A wrong figure is worse than none, so precision comes
first.

```sh
cargo build --release -p loker-core --example pay_text
cd eval/pay
python3 pay_eval.py sample 300                  # once: tuning set -> sample.json
python3 pay_eval.py sample 150 --set holdout3   # a fresh holdout (unseen jobs only) -> holdout3.json
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

Failures to work on next round (then draw `holdout3`):
- Wrong figures: "$" alone reads as USD in Canadian postings ("Ontario
  based: $80,000 - $100,000", "$20.00/hour"); "$1500 USD annually for
  professional development" is read as base ("development" isn't a
  benefit word); an OTE stated in a paragraph four lines above its figure
  is read as base (the OTE heading check looks two lines up).
- Missed: state-by-state "Min: $18.53, Max: $24.35" lists; "Rate: USD
  8.93 /hour" and "Monthly Rate: Php80,000 - Php100,000" ("rate" alone
  isn't a pay word); "Earn $100-$200 per hour"; "₱" and MYR; "100k€"
  (currency after a k figure); "$60,000,00" (a comma for decimals);
  "$185K–$215K base" (no pay word, no period); two ranges per line
  ("Metro 2: $130,000 - 150,000 Metro 3: ..."); and figures with no period
  ("Compensation: $3,000", "€1500 plus commission"), left out by design.
