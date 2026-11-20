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
LOKER_EVAL_DB=$PWD/snapshot-wwr.db python3 agent_eval.py run --label X+wwr   # another snapshot
```

## How it works

1. **Tasks** (`tasks.json`): a person, their country and UTC offset, and the
   role they want: "a data engineer in Berlin", "a DevOps engineer in Manila".
   The tasks run from scarce (a handful of eligible matches) to plentiful.
2. **Ground truth** per task, computed with the binary under test: every job
   loker marks yes or maybe for the viewer, listed with no query so ranking
   can't hide any, graded 0–3 for role fit by Opus 5.5 from title, company
   and the first 800 characters of the description. Good = grade ≥ 2.
   Grades are cached in `fit-labels.json`, keyed by task and job;
   `FIT_PROMPT` pins the prompt they were made with.

   Eligibility here is loker's own verdict, not a checked one: an
   eligibility bug that marks a job yes counts as a good pick, and a job
   loker wrongly marks `unknown` counts as a wrong one. This eval measures
   whether the agent finds what loker says the viewer can apply to; whether
   loker is right is tested in `crates/loker-core/tests` and `eval/golden`.
3. **Runs:** Pi with DeepSeek Flash and only loker's MCP tools, on a fresh
   copy of the snapshot. The prompt asks for up to 5 jobs and to say so when
   fewer qualify, and ends by asking for a `PICKS:` line of job ids.
4. **Scoring** from the `PICKS:` line and the session file.

## Metrics

| Column | Meaning |
|---|---|
| avail | Good jobs in the truth for the task |
| picks | Jobs the agent recommended |
| prec | Share of picks that are good (loker says yes or maybe, and Opus says the role fits); a run that picks nothing when good jobs exist counts 0 |
| rec@5 | Good picks / min(5, avail): did it find enough of what was there (coverage, not recall)? |
| pass | Every pick good and at least min(5, avail) of them; "none" passes only when avail is 0 |
| calls, kchars, ktok, $, sec | Tool calls, characters of tool output, tokens (cache reads included), DeepSeek cost, wall time |
| fail | Runs that errored, timed out, or had no `PICKS:` line |

Each run is one sample of a noisy agent: compare labels over 3+ trials per
task, and treat small differences as noise. With 5 tasks x 3 trials, one
run moves pass by 7 points, and the 3 runs of a task are not independent.

Limits:
- One judge (Opus) grades fit, with no second judge and no human check.
- The 5 tasks are also the ones the MCP descriptions were tuned on (the
  titles view and the list-everything guidance came from these runs).
  There is no held-out task set.
- `meta.json` (binary hash, version, snapshot, agent model) is recorded
  from now on; earlier runs are named after the git HEAD, but `--bin` could
  have been any build.
- The latest runs predate typed pay (388a92a) and visa (24880fa), which
  added fields to every result: the characters and tokens below are not
  what an agent reads today.

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
| 61b0dc6 (+ totals, titles view, list-everything guidance) | 9.9 | 88 | 143 | 0.014 | 24 | 92% | 92% | 87% |

No run in either label used the `seniority` filter, so the pass gap is not
from that change: 2 of 3 earlier data-de runs recommended jobs with an
unknown verdict, none of the later ones did. With 3 trials, that is noise.

What the runs show:
- Precision fails when the agent recommends `unknown` jobs as eligible.
- Scarce tasks cost the most: confirming that nothing qualifies took 15-18
  calls and 140-175K characters, against 7-8 calls and 55-60K when matches
  exist. A way to see how many eligible jobs exist for a role would cut that.

After totals and the titles view (61b0dc6): a third fewer tokens (215K to
143K a run) and a quarter less cost, mostly on the scarce tasks (data-de 18
to 13 calls, devops-ph 15 to 9). Agents used `view: "titles"` in 52 of 149
calls. The two wrong picks were eligible jobs Opus judged off-role, not
eligibility errors.

### Snapshot 2026-11-19 + We Work Remotely (1,224 + 455 listings)

The search eval's snapshot with `loker ingest weworkremotely` added
(`snapshot-wwr.db`, local only). Truth: backend-id 51 good of 412 eligible,
ml-in 20/389, devops-ph 21/395, data-de 13/372, designer-gb 6/391.

| Label | calls | kchars | ktok | $/run | sec | prec | rec@5 | pass |
|---|---|---|---|---|---|---|---|---|
| 61b0dc6 | 6.3 | 54 | 67 | 0.009 | 19 | 97% | 97% | 87% |

Every task now has jobs to give, and runs are cheaper than on the original
snapshot: a task with matches needs 4-7 calls. designer-gb (6 good of 391)
fails 2 of 3 runs on one off-role pick each.

### Snapshot 2026-11-19 b (1,749 jobs, fetched with source categories)

`eval/role/snapshot.db`. Truth: backend-id 57 good of 468 eligible, devops-ph
28/451, ml-in 22/444, data-de 16/422, designer-gb 7/442.

| Label | calls | kchars | ktok | $/run | sec | prec | rec@5 | pass |
|---|---|---|---|---|---|---|---|---|
| 93c1c9f (before roles) | 5.6 | 51 | 57 | 0.009 | 17 | 100% | 97% | 93% |
| 1a0922e (roles, `role` filter, list_roles) | 4.5 | 32 | 37 | 0.006 | 15 | 100% | 100% | 100% |
| 163403f + Jev answers (`eval/role/snapshot-jev.db`) | 4.7 | 30 | 36 | 0.006 | 14 | 100% | 99% | 93% |

Agents used the `role` filter in 23 of 68 calls (`data_engineering`,
`backend`, design and AI roles) and list_roles twice. Characters read fell
38% and tokens 36%. designer-gb, the scarcest task (7 good jobs), went from
2 of 3 runs passing to 3 of 3: one run, within noise.

With Jev's roles (79% exact against 68%) the agent numbers stay the same:
these tasks' good jobs have titles that name the role, which the rules
already get right. The one failure is a design run that stopped at 4 of 7
good jobs after searching only `minVerdict: yes`; all 7 carry a design role.
Jev's gain shows on jobs with vague titles, which these tasks rarely need.

