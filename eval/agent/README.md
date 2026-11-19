# Agent eval

Measures what an agent actually gets from loker: does it find the jobs the
viewer can apply to, and how much does it read to get there. Run it by hand
when changing anything an agent sees (tool descriptions, result shapes,
ranking, filters); it isn't part of CI. The search eval (`../search`)
measures ranking alone and is cheaper; use this one for end-to-end effects.

Needs Pi, `DEEPSEEK_API_KEY` (the agent), the `claude` CLI (the judge), a
release build, and the search eval's `snapshot.db`.

```sh
cargo build --release
cd eval/agent
python3 agent_eval.py run                      # 5 tasks x 3 trials on target/release/loker
python3 agent_eval.py run --bin /path/to/old/loker --label before
python3 agent_eval.py report                   # every label in runs/
```

## How it works

1. **Tasks** (`tasks.json`): a person, their country and UTC offset, and the
   role they want: "a data engineer in Berlin", "a DevOps engineer in Manila".
   The tasks run from scarce (a handful of eligible matches) to plentiful.
2. **Ground truth** per task, computed with the binary under test: every job
   the viewer can apply to (verdict yes or maybe), listed with no query so
   ranking can't hide any, graded 0–3 for role fit by Opus 5.5 from title,
   company and the start of the description. Good = grade ≥ 2. Grades are
   cached in `fit-labels.json`.
3. **Runs:** Pi with DeepSeek Flash and only loker's MCP tools, on a fresh
   copy of the snapshot. The prompt asks for up to 5 jobs and to say so when
   fewer qualify, and ends by asking for a `PICKS:` line of job ids.
4. **Scoring** from the `PICKS:` line and the session file.

## Metrics

| Column | Meaning |
|---|---|
| avail | Good jobs in the truth for the task |
| picks | Jobs the agent recommended |
| prec | Share of picks that are good (eligible and fit the role) |
| rec@5 | Good picks / min(5, avail): did it find what was there? |
| pass | Every pick good and at least min(5, avail) of them; "none" passes only when avail is 0 |
| calls, kchars, ktok, $, sec | Tool calls, characters of tool output, tokens (cache reads included), DeepSeek cost, wall time |
| fail | Runs that errored, timed out, or had no `PICKS:` line |

Each run is one sample of a noisy agent: compare labels over 3+ trials per
task, and treat small differences as noise.

## Results log

### Snapshot 2026-11-19 (1,224 active jobs), DeepSeek Flash via Pi, 3 trials

Truth: backend-id 11 good of 114 eligible, ml-in 10/91, designer-gb 1/108,
data-de 0/89, devops-ph 0/97. Every real data engineering job says "remote"
without a region (verdict unknown for DE), and no DevOps job is open to the
Philippines: coverage, not search, limits those tasks.

| Label | calls | kchars | ktok | $/run | sec | prec | rec@5 | pass |
|---|---|---|---|---|---|---|---|---|
| 05354ce (stemming, title weight, partial) | 13.1 | 119 | 210 | 0.018 | 29 | 82% | 85% | 73% |
| 63a4376 (+ seniority) | 12.6 | 120 | 215 | 0.018 | 29 | 100% | 100% | 100% |

No run in either label used the `seniority` filter, so the pass gap is not
from that change: 2 of 3 earlier data-de runs recommended jobs with an
unknown verdict, none of the later ones did. With 3 trials, that is noise.

What the runs show:
- Precision fails when the agent recommends `unknown` jobs as eligible.
- Scarce tasks cost the most: confirming that nothing qualifies took 15-18
  calls and 140-175K characters, against 7-8 calls and 55-60K when matches
  exist. A way to see how many eligible jobs exist for a role would cut that.
