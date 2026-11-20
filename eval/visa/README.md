# Visa eval

Measures visa sponsorship read from job descriptions
(`crates/loker-core/src/visa_text.rs`) against sponsorship labelled by
Opus and DeepSeek. Run it by hand when changing those rules; it isn't part
of CI. A wrong "not offered" turns a job away from someone who could take
it, so precision comes first.

```sh
cargo build --release -p loker-core --example visa_text
cd eval/visa
python3 visa_eval.py sample                      # once: sample.json (tuning), holdout.json
python3 visa_eval.py sample2                     # once: holdout2.json
JUDGE=opus python3 visa_eval.py label --set NAME # cached; JUDGE=deepseek needs DEEPSEEK_API_KEY
python3 visa_eval.py agree --set NAME
python3 visa_eval.py score --set NAME [-v]       # -v lists misses (not for a holdout before it is scored)
```

## What counts

Rulings (2026-11-20), in the labelling prompt (v1) and the rules:
- `offered`: the employer sponsors or supports the visa ("Visa sponsorship
  is available", "We'll cover your visa sponsorship", a benefits list
  with "visa sponsorship").
- `limited`: some cases only ("may", "case by case", "eligible
  candidates"), transfers only, or sentences that disagree by place.
- `not_offered`: won't sponsor, or the hire must not need sponsorship.
- Absent: nothing about sponsorship. "Must be authorized to work in the
  US" alone is absent: it doesn't say whether the employer sponsors.
  Citizenship and clearance requirements are absent too; they are a
  separate field, later.

## Sets

| set | from | jobs | mention visa words | state sponsorship |
|---|---|---|---|---|
| `sample.json` (tuning) | `eval/pay/snapshot.db`, 2026-11-19 | 200 | 133 | 96 |
| `holdout.json` | `eval/role/snapshot-fresh.db`, the role eval's 300 fresh jobs (golden set's 12 left out) | 150 | 36 | 4 |
| `holdout2.json` | the fresh snapshot's jobs in no set, at companies not in the old snapshot, at most 3 per company | 63 | 20 | 2 |

Labelers agree on 198/200, 150/150 and 63/63. The two tuning
disagreements are "may not be able to ... support future H-1B
sponsorship" (Opus `limited`, DeepSeek `not_offered`); the rules say
`limited`.

## Results

| set | claims right | stated found | wrong claims |
|---|---|---|---|
| tuning (Opus) | 96/96 | 96/96 | 0 |
| tuning (DeepSeek) | 94/96 | 94/96 | the 2 above |
| `holdout`, scored once | 4/4 | 4/4 | 0 |
| `holdout2`, scored once | 1/1 | 1/2 | 0 |

The holdouts are too small to give a rate. Unseen postings stated
sponsorship 6 times, all `not_offered`, and the rules found 5 with no
wrong claim, and made no claim on any of the 207 holdout jobs that say
nothing. `offered` and `limited` have no unseen test at all.

Wording in the tuning snapshot is heavily templated ("Visa sponsorship is
not available." in dozens of jobs), and sponsorship statements are rare at
companies outside it: of 540 fresh jobs at new companies that mention visa
words, 9 companies hold all of them.

The `holdout2` miss was "### VISA Sponsorship:" with "No" on the next
line. It is fixed (a heading is read with the line under it), so
`holdout2` is burned; it scores 2/2 now, which is not a result. Jobs read
while writing rules are in `seen.json`; every tuning job was read.

Across the tuning snapshot the rules state sponsorship for 264 of 1,749
jobs: 177 not offered, 82 offered, 5 limited. Each read adds about 0.5 ms
per job.

To do: a holdout with `offered` and `limited` postings from boards loker
has not ingested.
