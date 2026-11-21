# Search-ranking eval

Measures whether search puts the right jobs first. Run it by hand when changing
ranking (tokenizer, weights, query handling); it isn't part of CI or `nix flake
check`. Needs Python 3 (stdlib only) and, for new grades, the `claude` CLI
(Opus judge) or `DEEPSEEK_API_KEY` (second judge).

```sh
cd eval/search
python3 eval.py snapshot                 # once: freeze $LOKER_HOME/loker.db -> snapshot.db
python3 eval.py pool                     # run every variant, top 20 per query
JUDGE=opus python3 eval.py judge         # grade pairs not yet graded (cached)
JUDGE=opus python3 eval.py score -q      # metrics; -q per query, --holdout for the holdout set
                                         # --base VARIANT: Δ against that variant, not the baseline
python3 eval.py agree                    # Opus vs DeepSeek agreement
python3 eval.py sample 40                # blind sheet for a human spot-check
```

## How it works

1. **Snapshot.** A frozen copy of a real database (`snapshot.db`, not committed:
   it's large and mostly other sites' job text). Every variant ranks the same
   jobs. The eval builds its own FTS5 tables from it (`eval_plain`,
   `eval_porter`), so it doesn't depend on which tokenizer loker ships.
2. **Queries** (`queries.json`). Each has the text an agent or person would
   type and an `intent` the judge grades against. `holdout: true` queries are
   kept out of tuning; `expect_none: true` queries should return nothing.
3. **Variants** (`VARIANTS` in `eval.py`). One ranking each, as SQL in the same
   shape as `store.rs`. The baseline `plain and` is loker's default search as
   of 58be2e1. loker now ships `porter and->or t5` as its default and
   `porter or t5` as `anyWord`.
4. **Pool.** The union of every variant's top 20 per query. Only pooled pairs
   are graded, so recall means "of the relevant jobs any variant found".
5. **Judge.** Each (query, job) gets 0–3: 3 exactly the role sought, 2 clearly
   relevant, 1 tangential, 0 not relevant. Opus 5.5 (`claude -p`, 10 jobs a
   call, no tools, an empty working directory so no CLAUDE.md reaches it) is the main judge; DeepSeek grades
   the same pairs as a check. Grades are cached in `labels-*.json`, keyed by
   prompt version, model, query, intent and job id: a new variant only grades
   its new pairs, and editing a query's intent re-grades that query.
   `PROMPTS` pins the prompt text and truncation each version names, so
   `judge` refuses to run after a prompt edit until the version is bumped.

## Metrics

Relevant means grade ≥ 2.

| Metric | Question |
|---|---|
| **nDCG@10** (headline) | Are the best jobs at the top? Higher grades and higher positions count more; 1.0 is the ideal order. |
| P@10 | What share of the top 10 is relevant? |
| R@20 | What share of the pool's relevant jobs is in the top 20? |
| empty | Queries with no results. |
| results, junk | Results returned per query, and how many of the top 20 aren't relevant: what an agent reads for nothing. |
| unjudged | Results with no grade; should be 0. |
| should-match-nothing | Results returned / judged relevant for `expect_none` queries; ideal `0/0`. |

The Δ column is the nDCG gain over the baseline with a 95% interval from a
paired bootstrap over queries. With ~30 queries only large gains are reliable;
confirm a winner on `--holdout` before shipping it.

## Hill-climbing

- **New idea:** add a variant, run `pool`, `judge`, `score`. Ship it to loker
  only if it beats the current best on the tuning set and the holdout.
- **After shipping:** update step 3 of How it works so it says which
  variant loker now matches, and add a row to the results log.
- **New queries:** add them before looking at any variant's results for them.
  Don't tune toward the holdout.
- **New snapshot:** grades stay valid for job ids that still exist. Move the
  old `snapshot.db` away, take a new one, re-run `pool` and `judge`, and start a
  new section in the results log (numbers from different snapshots don't
  compare).
- **Judge trust:** before relying on a new judge or prompt, check `agree`
  (binary agreement ≥ 80%, kappa ≥ 0.5) and grade a `sample` yourself.

Not measured here: eligibility (tested in `crates/loker-core/tests`), jobs no
variant found, and agent behaviour end to end.

## Results log

### Snapshot 2026-11-19 (1,224 active jobs, 7 sources)

38 queries (30 tuning, 5 holdout, 3 expect-none), 1,305 graded pairs. Judges:
binary agreement 89%, kappa 0.79; Opus rates 51% relevant, DeepSeek 61%.

| Variant | nDCG@10 tuning | Δ [95% CI] | nDCG@10 holdout | R@20 tuning |
|---|---|---|---|---|
| plain and (loker 58be2e1 default) | 0.504 | | 0.485 | 0.42 |
| plain or (58be2e1 `anyWord`) | 0.565 | +0.061 [+0.005, +0.119] | 0.639 | 0.59 |
| plain and->or t5 | 0.617 | +0.113 [+0.057, +0.172] | 0.656 | 0.61 |
| porter or t5 | 0.679 | +0.174 [+0.094, +0.256] | 0.725 | 0.71 |
| porter and->or t5 (shipped) | 0.681 | +0.177 [+0.098, +0.258] | 0.709 | 0.71 |
| syn porter and->or t5 | 0.653 | +0.148 [+0.038, +0.249] | 0.695 | 0.71 |

The shipped variant and `porter or t5` can't be told apart: +0.003
[−0.002, +0.008] on tuning, and on the holdout `porter or t5` is ahead
(−0.016 [−0.070, +0.021] for the shipped one). It was shipped for showing
full matches first, not for its score, which breaks the rule above
("beats the current best on the tuning set and the holdout").

The holdout is 5 queries, and every variant here was scored on it, in two
rounds: its intervals are about ±0.2, and it is no longer unseen. A new
ranking needs new holdout queries, scored once.

Findings:
- Stemming helps: +0.064 [+0.011, +0.125] with partial-match filling,
  +0.076 [+0.017, +0.148] without ("data engineer" 0.18 → 0.68, "security engineer" 0.12 →
  0.91). Filling with partial matches helps about as much: +0.086
  [+0.036, +0.142] (`plain and t5` → `plain and->or t5`).
- A title weight of 5 helps when any word matches (+0.037 [+0.003,
  +0.078]); with every word it is within noise (+0.027 [−0.005, +0.062]).
  10 is no better than 5.
- AND, then filling with OR, removes empty results for long agent queries, but
  `air traffic controller` then returns 20 irrelevant jobs. Results filled
  from OR should be marked as partial matches.
- The naive synonym map is mixed: it helps `sre` and `frontend react` but
  `backend` → "back end" drops 0.95 → 0.28. Synonyms need a curated,
  measured list.
- Weak spots: location words in queries ("remote … asia timezone"), and
  seniority as a keyword ("junior developer" gets worse with stemming).

Shipped: stemming, title weight 5, and every-word-first with `partial`
marks. `loker search` on this snapshot returns the eval's `porter and->or t5`
lists exactly (and `--any-word` the `porter or t5` lists) for all 38 queries.
The MCP description now tells agents to keep location out of `q`.

Tried and not shipped: adding partial matches only when few jobs have every
word. Partial matches are often relevant (a good job without "remote"), so
cutting them costs more than it saves:

| Variant | nDCG@10 tuning | holdout | R@20 | results | irrelevant |
|---|---|---|---|---|---|
| porter and->or t5 (shipped) | 0.681 | 0.709 | 0.71 | 19.5 | 6.7 |
| porter and->or<5 t5 | 0.669 | 0.677 | 0.67 | 17.6 | 5.3 |
| porter and->or<1 t5 | 0.652 | 0.677 | 0.63 | 16.0 | 4.8 |
