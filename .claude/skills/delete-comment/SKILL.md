---
name: delete-comment
description: Audit code comments in loker (all of it, or a path you name), sort each into tautology, stale, excuse or decision record, then delete it, file an issue for it, or move it into the docs, and lower the comment ratchet. Use when asked to delete or clean up comments, run a comment pass, or when lint/comment_ratchet.py fails on a new comment.
---

# /delete-comment

loker doesn't keep comments in its code. `lint/comment_ratchet.py` counts
comment lines per file against `lint/comment-baseline.txt` and `nix flake check`
fails if a file gains one. The count only goes down: each pass deletes some
comments and lowers the baseline.

Arguments: optional paths (`crates/loker-core/src/store.rs`, `eval/`). With
none, take the files with the most comment lines in the baseline.

## 1. List

```sh
python3 lint/comment_ratchet.py --list [PATH ...]
```

Work only from this list. It already leaves out text that only looks like a
comment, and must not be touched:

- doc comments on types deriving `JsonSchema` (they become the descriptions in
  `schema/v1`, which agents read over MCP) or clap's `Parser`, `Subcommand`,
  `Args`, `ValueEnum` (they become `--help`);
- Python module docstrings printed with `sys.exit(__doc__)` (usage text);
- shebangs and tool directives (`# noqa`);
- `crates/loker-core/migrations/`: sqlx checksums shipped migrations, so
  editing one breaks every existing database.

## 2. Sort each comment block

Read the code around it, then pick one:

| Kind | Looks like | Action |
|---|---|---|
| Tautology | Says what the name, signature or next line says. Section dividers. | Delete. |
| Stale | Describes code or behaviour that has changed. Check before deciding. | Delete. |
| Excuse | Justifies a shortcut: "for now", "good enough", "TODO", "hack", a known-wrong case tolerated. | Delete and file an issue (step 3). |
| Decision record | Why it is this way: a trade-off, an outside constraint (a source's API quirk), a subtlety someone would "fix" wrongly. | Move it (step 4). |

Be strict. Most function doc comments restate the name. When a doc comment
mixes a restatement with one non-obvious clause, judge by that clause.

## 3. Excuses become issues

One issue per hack (group only when it is one fix in several places):

```sh
gh issue create --label comment-debt --title "<what to fix>" --body "<body>"
```

The body names the file and function, quotes the deleted comment, says what
the shortcut is and suggests a fix. Create the `comment-debt` label first if
it doesn't exist (`gh label create comment-debt`).

## 4. Decision records go into the docs that exist

There is no ADR directory; don't make one. Put each record where a
contributor already looks:

- the pipeline, rules, trade-offs and how each part works: `ARCHITECTURE.md`
  (*Rules*, *Trade-offs*, *How it works*, *Known exceptions*);
- running, operating, CI and the cache: `README.md`;
- how an eval samples, labels or scores: that eval's `README.md`;
- what a test expects and why: the test's assertion message.

If it is already written there, just delete the comment. Write in the docs'
own style: short plain sentences, the decision and its reason. A subtlety that
would mean nothing in prose may stay for a later pass; the ratchet allows it.

## 5. Check and lock in

```sh
python3 lint/comment_ratchet.py --update   # lower the baseline
python3 lint/comment_ratchet.py --check
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
cargo run -- schema --out schema/v1 && git diff --exit-code schema/  # must not change
```

If `schema/v1` changed, a deleted doc comment was a schema description:
restore it, and fix the lister if it printed it. Commit as
`refactor: delete comments in <area>`, with the excuse issues and moved
records listed in the body.
