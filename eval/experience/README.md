# Experience eval

Measures the years of experience read from job descriptions
(`crates/loker-core/src/experience_text.rs`) against the minimum years
labelled by Opus and DeepSeek v4.1 (Pi, OpenCode Go, high thinking). Run it
by hand when changing those rules; it isn't part of CI.

A number too high hides a job from someone who meets the real bar (the
`experienceYears` filter drops jobs asking for more), so **too high** is
the number to keep near zero. A miss only leaves a job unfiltered.

```sh
cargo build --release -p loker-core --example experience_text
cd eval/experience
python3 experience_eval.py sample                       # once: sample.json, holdout.json, holdout2.json
python3 experience_eval.py sample2                      # once: holdout3.json, holdout4.json
python3 experience_eval.py sample3                      # once: holdout5.json, holdout6.json
JUDGE=opus|deepseek41 python3 experience_eval.py label --set NAME   # cached
python3 experience_eval.py agree --set NAME
python3 experience_eval.py score --set NAME [-v]        # -v lists misses, only for tuning or burned sets
```

## What counts

Rulings (2026-11-21), in the labelling prompt (v1) and the rules:
- `min_years`: the minimum required for this role; 0 when the posting says
  no experience is needed; absent when no number is stated ("several
  years", "extensive experience").
- Required only: years under a preferred heading or marked preferred, a
  plus or an asset don't count.
- Several required numbers (overall and per skill): the largest.
  Alternatives (degree paths) and levels ("For Senior: 6+"): the smallest.
- Not the hire's: a company's age, a contract or degree length, someone
  else's story, "we don't need 10+ years".

Y Combinator states it in a field (`minExperience`), mapped at ingest and
not measured here: "Any (new grads ok)" is 0, "3+ years" is 3.

## Sets

| set | from | jobs | companies |
|---|---|---|---|
| `sample.json` (tuning) | `eval/role/snapshot-fresh.db` | 200 | 118 |
| `holdout.json` | `eval/visa/snapshot-boards.db`, 17 employer boards (visa's holdout3 left out) | 150 | 19 |
| `holdout2.json` | `eval/eligibility/snapshot-workable4.db`, Workable's search | 100 | 66 |
| `holdout3.json` | `eval/visa/snapshot-boards2.db`, 19 more boards | 100 | 18 |
| `holdout4.json` | `eval/eligibility/snapshot-h2.db`, no company from earlier sets | 150 | 66 |
| `holdout5.json` | `eval/eligibility/snapshot-workable3.db`, no company from earlier sets | 100 | 74 |
| `holdout6.json` | `eval/pay/snapshot.db`, employer boards, no company from earlier sets | 18 | 1 (Palantir) |

Each set is about 70% jobs whose text has a years-like phrase and 30%
others, so misses outside the net show. The labellers agree on
`min_years` for 198/200, 149/150, 99/100, 99/100, 147/150, 99/100 and
18/18. The disagreements are gray: Cloudflare's "Desirable Skills" list
(Opus required, DeepSeek preferred), "you don't need years of closing
experience" (0 or nothing).

## Results

Each holdout was scored once with the rules of its time, then read and so
became tuning (`burned.json`). Opus labels; DeepSeek's are within one job.

| set | rules | right | too high | missed |
|---|---|---|---|---|
| tuning | first | 129/130 | 0 | 6 |
| `holdout`, unseen | first | 73/87 | **14** | 8 |
| `holdout2`, unseen | first | 47/53 | **6** | 4 |
| `holdout3`, unseen | after holdout/holdout2 fixes | 52/53 | 1 | 14 |
| `holdout4`, unseen | same | 93/97 | **4** | 9 |
| `holdout5`, unseen | after holdout3/holdout4 fixes | 65/70 | **5** | 4 |
| `holdout6`, unseen | same | 10/10 | 0 | 1 |

The first holdouts failed on what the tuning snapshot didn't have: Qonto's
"your data is kept for up to 2 years" on every posting, companies' own age
("drawing on 30+ years' experience"), "or" read as an alternative in
"program or project management", and "ideal" headings read as preferred.
The last unseen sets (`holdout5`, `holdout6`) had 5 too high in 80
claims: a company's 20 years (the cap was then above 20), "we don't need
10+ years", and two gray postings that state the years both as a
requirement and as preferred. The first two are fixed; the fix has no
unseen test.

With the current rules, across all sets (all now tuning): too high 0, 1, 0,
0, 1, 2 and 0; the misses are mostly French, German and Indonesian postings
("au moins 3 ans", "mindestens fünf Jahre"), which the rules don't read.

Across the fresh snapshot's 4,180 jobs the rules state years for 2,540
(61%): Greenhouse 62%, Ashby 57%, Lever 52%, We Work Remotely 65%, Jobicy
65%, Himalayas 70%, Remote OK 50%. Reading costs about 0.3 ms a job.
