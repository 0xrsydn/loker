# Golden set

24 jobs checked by hand, blind: 12 for pay from `eval/pay/holdout3.json`,
12 for role from `eval/role/fresh.json`. Each job showed its distinct
answers (Opus's label, DeepSeek's label, loker's output) with no source
named; the reviewer marked each right or wrong.

```sh
python3 review.py build    # the 24 jobs -> items.json (not committed; rebuilt identically)
python3 review.py serve    # the review page on 127.0.0.1:8787
python3 review.py score
```

`verdicts.json` holds the marks. Marks listed under `set_in_chat` were
agreed in conversation (2026-11-20) for answers left unmarked on the page.

## Rulings

Decisions on the spec made during the review, for the labelling prompts and
the rules:
- **Pay: base pay is what counts.** An answer that misses an extra but has
  base pay right is right (Mercor's "Generous equity grant", no figure).
- **Relocation bonus** ("Up to $15K relocation bonus", paid to hires who
  move) is a bonus extra, conditional on moving.
- **"ownership opportunities and dividends for long-term employees"** is
  not equity offered to this hire.
- **"Software Engineer, <area>"**: the area gives the role when it is a
  discipline ("Machine Learning", "Infrastructure", "Mobile"); a product area
  ("Consumer Revenue", "Discovery") does not.
- **No fitting role is `none`**: Spotify's "Senior Music Editor" (playlist
  curation) has no role in the vocabulary. A content or editorial role is a
  gap to fill if it recurs.
- **An extra or missing "also" role** doesn't make an answer wrong when the
  main role is right.

## Results (2026-11-20)

| | pay (12) | role (12) |
|---|---|---|
| Opus | 11 | 9 |
| DeepSeek | 11 | 12 |
| loker (pay rules; role rules + Jev, shipped) | 12 | 11 |
| loker, role rules alone | | 9 |

What the hand check shows:
- Both labelers read equity into "ownership opportunities and dividends for
  long-term employees" (P8). Two models agreeing was wrong there.
- Opus, the labeler every headline number is scored against, was wrong on
  3 of 12 roles: a fullstack where the area is infrastructure, a fullstack
  for a product engineer, and a "founding" modifier.
- loker's pay output had the right figures on all 12, but one job stated
  its range twice and got two ranges with junk labels. Fixed in `d4175d1`.

Limits: 24 jobs. 3 of the 12 pay jobs share Mercor's template, and 7 of
the 12 have every source agree, so the pay half says little about hard
cases. Taken as a check on the labels, not as accuracy figures.
