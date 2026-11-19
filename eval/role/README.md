# Role eval

Measures the role classifier (`crates/loker-core/src/classify.rs`) against
jobs labelled by a model with the same vocabulary. Run it by hand when
changing the classifier's tables or the vocabulary; it isn't part of CI.

```sh
cargo build --release
cd eval/role
python3 role_eval.py sample 600              # once: pick jobs from snapshot.db -> sample.json
JUDGE=opus python3 role_eval.py label        # label unlabelled sample jobs (cached)
JUDGE=deepseek python3 role_eval.py label    # second labeler (needs DEEPSEEK_API_KEY)
python3 role_eval.py agree                   # how far the two labelers agree
python3 role_eval.py score [-v]              # classifier vs the Opus labels
python3 role_eval.py score --bin /path/to/old/loker
python3 role_eval.py jev                     # Jev's pick per job (needs OPENROUTER_API_KEY)
python3 role_eval.py score --pred jev        # score Jev; --pred hybrid:0.6 = rules for titles, Jev for the rest

# a holdout of jobs the rules never saw: boards and postings not in snapshot.db
LOKER_EVAL_DB=snapshot-fresh.db python3 role_eval.py sample 300 --set fresh --new-since snapshot.db
JUDGE=opus python3 role_eval.py label --set fresh       # and JUDGE=deepseek
python3 role_eval.py score --set fresh                   # rules; LOKER_EVAL_DB=<enriched copy> for rules + Jev
```

## How it works

1. **Snapshot** (`snapshot.db`, local only, not committed): a loker database
   fetched after sources started keeping their category (2026-11-19, 1,749
   active jobs). Each run copies it, so the binary under test classifies it
   on open with its own rules.
2. **Sample** (`sample.json`): 600 job ids drawn at random (seed 1), so the
   mix of sources is the snapshot's own.
3. **Labels** (`labels-opus.json`, `labels-deepseek.json`): each labeler sees
   the title, company, source category and the first 1,200 characters of the
   description, plus the vocabulary from `loker roles`, and answers primary,
   up to two secondary roles, modifiers, and a missing role when none fits.
   Keyed by prompt version, vocabulary version and job id: a new vocabulary
   version labels again.
4. **Score**: the binary's `role` for each sampled job against the Opus label.

## Metrics

| Metric | Meaning |
|---|---|
| covered | Jobs the classifier gives a role |
| exact | Same primary id as the label |
| findable | A filter on the label's role returns the job (`JobRole::matches`): our primary or a secondary sits inside it. Strict; use this or exact |
| compatible | Same id, one inside the other in either direction, or the label's primary among our roles. Generous: tagging every job `engineering` scores 36%, so never a headline |
| same top family | Right family, even if the role differs |
| missed / false role | No role where the label has one / a role where the label says none |
| by method | The above for title, category and description decisions |
| modifiers | Precision and recall of forward_deployed and founding |

The incompatible list (label → ours, with titles) is where rules go wrong;
"roles the judge found missing" is where the vocabulary may need to grow.

## Results log

### Snapshot 2026-11-19 (1,749 jobs), vocabulary v1, rules v1

Labelers agree: same primary for 90% of the 600 jobs, same top family 96%.
17 jobs fit no role (line cooks, screenwriters, estimators).

| Predictor | covered | exact | compatible | same family |
|---|---|---|---|---|
| rules (classify.rs) | 97% | 68% | 73% | 83% |
| Jev 1.14 choice, alone | 99% | 80% | 84% | 91% |
| rules for specific titles, Jev for the rest | 99% | 78% | 82% | 90% |
| Jev, rules where Jev < 0.6 and the title is specific | | 81.5% | | |

By what the rules decided from (exact, rules vs Jev on the same jobs):

| Decided by | jobs | rules | Jev |
|---|---|---|---|
| specific title phrase | 425 | 83% | 85% |
| source category | 83 | 14% | 64% |
| generic "engineer" title | 62 | 56% | 71% |
| nothing | 18 | 17% | 83% |

Findings:
- Title rules are as good as Jev where a title names the role; categories
  are where rules fail (Clera files every job, GTM included, under
  "Engineering"), and so are generic "Software Engineer" titles whose
  description says what the job is.
- Modifiers: rules have 100% precision, 79% (forward_deployed) and 90%
  (founding) recall; Jev's single choice doesn't give them.
- Jev costs $0.045 for 600 jobs with an 800-character description.
- The judges found these roles missing: deployment strategist (7),
  community management, product operations, GTM engineering, workforce
  management, video editing (2 each).

### Shipped: Jev in loker (rules v2, `loker roles enrich`)

`snapshot-jev.db` is `snapshot.db` after `loker roles enrich` with the real
API: 1,588 distinct jobs asked, none failed, $0.12, 2m42s. Scored with the
binary as shipped (Jev unless a title names the role and Jev is under 0.6):

| | covered | exact | compatible | same family |
|---|---|---|---|---|
| rules only (`snapshot.db`) | 97% | 68% | 73% | 83% |
| rules + Jev (`snapshot-jev.db`) | 99% | 79% | 84% | 90% |

The rest are mostly judgment calls: Palantir's "Forward Deployed Software
Engineer" goes to sales_engineering (Opus: software), "Founding Engineer" at
AI startups to ai_engineering, and Deployment Strategist scatters (both
labelers flagged it as a missing role).

### Audit (2026-11-19): a holdout the rules never saw

The numbers above are all on `sample.json`, which the classifier's tables
were tuned against (its incompatible lists drove the rule changes) and on
which Jev's 0.6 override threshold was picked. Before rules v2 was
committed, 1,222 of the snapshot's 1,749 titles had been printed during
development, so no clean holdout exists inside `snapshot.db`.

`fresh.json` is one: 300 jobs from `snapshot-fresh.db`, fetched after
rules v2 was frozen from 13 company boards loker had never ingested
(Stripe, OpenAI, Cloudflare, Ramp, Notion, Reddit, ...) plus the
aggregators, keeping only postings whose URL is not in `snapshot.db`, at
most 40 per source. It leans to company boards (few aggregator jobs were
new), so it has more specific titles than the snapshot. Labelled by Opus
and DeepSeek (89% same primary); scored once. Rules + Jev is the shipped
binary on `snapshot-fresh-jev.db`, a copy where only these 300 jobs are
active, after a real `loker roles enrich` (296 distinct jobs, $0.02).

| `fresh.json`, 300 jobs | exact (Opus / DeepSeek) | findable | same family | tuning sample, exact |
|---|---|---|---|---|
| rules only | 71% / 70% | 73% / 72% | 86% / 86% | 68% |
| rules + Jev, shipped | 83% / 81% | 84% / 82% | 92% / 90% | 79% |
| Jev alone (its cached answers) | 83% / 81% | | | 80% |

- No sign the tuning numbers were inflated: fresh jobs score as well or
  better, under either labeler.
- The override (rules where a title names the role and Jev is under 0.6)
  decides 26 fresh jobs: rules get 12 right, Jev 11 (Opus) or 12
  (DeepSeek). Its 1.5-point gain on the tuning sample was an artifact of
  picking the threshold there; on fresh data it is neutral.
- Strict findable on the tuning sample: rules 70%, rules + Jev 80% (the
  "compatible" 73% / 84% counted broader tags as hits).
- Scorer checks: the labels as predictions score 100%; no roles scores
  3% (the 17 jobs labelled none); everything `engineering` scores 0%
  exact and findable, 36% compatible.

