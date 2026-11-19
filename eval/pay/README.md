# Pay eval

Measures pay read from job descriptions (`crates/loker-core/src/pay_text.rs`)
against pay labelled by a model. Run it by hand when changing those rules;
it isn't part of CI. A wrong figure is worse than none, so precision comes
first.

```sh
cargo build --release -p loker-core --example pay_text
cd eval/pay
python3 pay_eval.py sample 300               # once: tuning sample -> sample.json
python3 pay_eval.py sample 150 --holdout     # once: never tuned on -> holdout.json
JUDGE=opus python3 pay_eval.py label         # label unlabelled jobs (cached); --holdout for the holdout
JUDGE=deepseek python3 pay_eval.py label     # second labeler (needs DEEPSEEK_API_KEY)
python3 pay_eval.py agree                    # how far the two labelers agree
python3 pay_eval.py score [-v]               # rules vs the Opus labels; --holdout for the holdout
python3 pay_eval.py score --bin /path/to/old/pay_text
```

## How it works

1. **Snapshot** (`snapshot.db`, local only): the role eval's snapshot
   (2026-11-19, 1,749 active jobs).
2. **Samples**: two thirds from jobs whose description has a currency next
   to a figure (1,028 of them), one third from the rest, so misses outside
   that net show too. `sample.json` (300) is for tuning; `holdout.json`
   (150, disjoint) is scored once per round of changes and never tuned on.
   After a round looks at its failures, the next round needs a new holdout.
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

### Snapshot 2026-11-19, prompt v1, rules v1

Labelers agree on base pay for 97% of the 300 tuning jobs (141 have none
by both; 6 where one says none).

| | jobs | base precision | base recall | jobs exact | wrong figure | extras P / R |
|---|---|---|---|---|---|---|
| first draft, tuning sample | 300 | 92% | 79% | 81% | 10 | 97% / 48% |
| shipped, tuning sample | 300 | 100% | 97% | 94% | 0 | 95% / 68% |
| shipped, **holdout** | 150 | **97%** | **87%** | 89% | 2 | 92% / 78% |

The holdout is the honest number; the tuning sample is what the rules were
fitted to. Across the snapshot, base pay goes from 366 jobs (source fields)
to 930 (53%): Lever 0 → 234, We Work Remotely 0 → 259, Ashby +51,
Remote OK +17. Greenhouse descriptions rarely carry pay (+1); its pay is
in a separate field the board API returns per job.

Holdout failures, for the next round:
- Wrong: "$20.00/hour" in a Canadian posting is CAD to the labeler; "$"
  alone reads as USD. An OTE stated in a paragraph four lines above the
  figure ("On-Target Earnings (OTE) … listed below") is read as base; the
  OTE heading check looks two lines up.
- Missed: currencies not in the list (MYR), "Monthly Rate: Php80,000 -
  Php100,000" ("rate" alone isn't a pay word), "$60,000,00" (a comma for
  decimals), "$185K–$215K base" (no pay word and no period), two ranges
  per line ("Metro 2: $130,000 - 150,000 Metro 3: ..."),
  and stated figures with no period ("Compensation: $3,000",
  "€1500 plus commission"), which the rules leave out by design.
