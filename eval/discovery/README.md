# Discovery eval

Measures what `loker discover` finds: how many candidate boards are real,
whether the company name a check stores is the company, and how rich the
jobs on discovered boards are next to configured boards and aggregators.
Run it by hand after changing a finder, `Board::from_url`, `check_board` or
an employer-board adapter; it isn't part of CI. It sends thousands of
requests to the Greenhouse, Ashby and Lever APIs, so run it rarely.

```sh
export LOKER_HOME=$(mktemp -d)
loker discover                                   # newest Common Crawl, every board checked
loker discover --from forwarddeployed.txt        # optional second finder
cd eval/discovery
python3 discovery_eval.py boards $LOKER_HOME/loker.db [--fd companies.json] [--sample 30]
python3 discovery_eval.py sample $LOKER_HOME/loker.db --per-ats 40 > /tmp/rich/config.toml
LOKER_HOME=/tmp/rich loker ingest --all && LOKER_HOME=/tmp/rich loker sources eval --json > rich.json
python3 discovery_eval.py richness configured=base.json discovered=rich.json
```

`--fd` is forwarddeployed.fun's `data/companies.json`, whose names are kept
by hand: the ground truth for names. `base.json` is `sources eval --json`
over a snapshot of configured boards and aggregators
(`eval/eligibility/snapshot-h2.db`). Copy a database with `sqlite3 .backup`,
never `cp`, while loker may be writing it.

## Results, 2026-11-23

CC-MAIN-2026-43 plus forwarddeployed.fun's 509 boards. Every candidate was
checked: 17 minutes for the crawl's 5,582 at 4 requests in flight per host,
no 429, no failed check.

| ATS | found | live | empty | missing | jobs on live boards |
|---|---|---|---|---|---|
| Ashby | 1,237 | 1,155 | 43 | 39 | 36,161 |
| Greenhouse | 4,204 | 3,330 | 218 | 656 | 148,030 |
| Lever | 362 | 243 | 14 | 105 | not counted |
| all | 5,803 | 4,728 | 275 | 800 | 184,191 + Lever |

4,225 live boards came only from the crawl, 218 only from the list, 285 from
both. Missing boards are mostly renamed or deleted companies, and Lever slugs
the crawl spelled in another case.

### Company names

| ATS | named | website | slug already is the name |
|---|---|---|---|
| Ashby | 99% | 98% | 81% |
| Greenhouse | 100% | 0% | 68% |
| Lever | 98% | 0% | 67% |

A third of Greenhouse and Lever boards would show a wrong company if named
from the slug (`agiholdings` is American Growth Insurance, `hs` is
Headspace). On the 499 boards both finders share, the ATS's name equals the
hand-kept one on 95%; 5% differ by a suffix or tagline ("Palantir
Technologies", "Cognite - AI for Industry"); one differs outright
(Intercom's Greenhouse board says "Fin", its new product name). 14 live
boards have no name: their hosted page is turned off (Cursor, PostHog) or
didn't answer.

What the name can't fix:

- **Not a public board (about 27 Greenhouse boards, ~1,200 jobs):** internal
  mobility, referral-only, alumni, invitation-only and talent-community
  boards, and LinkedIn or third-party mirrors of a main board ("Referrals
  Only", "Stone - Linkedin", "Home Chef (Internal Employees)"). Their name
  is the board's purpose, not the company, and internal jobs aren't open to
  the reader. Since fixed: such a board is `private` and not ingested; the
  rule marks 28 of these boards (about 1,000 jobs) and no other.
- **Careers-page decoration (about 36):** "Kaseya Careers", "Careers at
  Eucalyptus", "Astera Labs Early Career". Since fixed: 42 names lose the
  wording or stray punctuation ("Seso Inc.," is "Seso Inc."); taglines after
  a dash ("Cognite - AI for Industry") stay, since a dash is also part of
  real names.
- **One name, several boards (40 names, 82 boards):** the same company on
  two ATSs after a move (Ramp: 187 jobs on Ashby, 1 left on Greenhouse;
  Kalshi, Miro, Etched) or two boards on one (BetterHelp), and different
  companies with one name (Paradigm, Radar, Zip and Zip Co, Electra).
  Ingested together they merged 99 listings, all within one company; no two
  different companies' jobs merged, because their titles differed.

### Richness

40 random live boards per ATS, ingested; shares weighted by jobs.

| group | boards | jobs | scope | pay | pay in USD | unmapped location |
|---|---|---|---|---|---|---|
| configured Ashby | 11 | 566 | 64% | 65% | 60% | 3% |
| discovered Ashby | 40 | 1,225 | 89% | 58% | 53% | 1% |
| configured Greenhouse | 28 | 2,580 | 15% | 56% | 41% | 1% |
| discovered Greenhouse | 40 | 962 | 8% | 34% | 23% | 2% |
| discovered Lever | 40 | 4,555 | 82% | 48% | 42% | 34% |
| Himalayas | 1 | 200 | 100% | 40% | 36% | 3% |
| We Work Remotely | 1 | 548 | 60% | 54% | 43% | 2% |

Every discovered job had a description and a posted date. Ashby is as rich
as the hand-picked boards. Greenhouse states no workplace, so 90% of its
jobs have an unknown scope wherever they come from; its discovered boards
are less often US tech companies, so they state pay less often. Lever's
unmapped share is two boards: Veeva writes "Massachusetts - Boston" (scope
unknown on 3 in 4 of its 860 jobs, though Lever's own `country` field says US), and
Bluelight lists Latin American cities the tables don't know. Since fixed
for the first kind: a remote Lever job whose location no rule reads takes
Lever's `country` (685 of the sample's jobs).
