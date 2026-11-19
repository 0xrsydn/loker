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
| compatible | Same id, one inside the other (`engineering` vs `backend`), or the label's primary among our roles: filters still find the job |
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

