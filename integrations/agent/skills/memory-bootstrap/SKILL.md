---
name: memory-bootstrap
description: Bootstrap an empty repo's comemory corpus — index code and docs, distill past sessions, then save the decisions the repo cannot derive from itself.
---
# Bootstrap comemory for this repo

## When to Use

The repo has no memories: the SessionStart nudge fired (`comemory-status.sh`
reported `additionalContext` naming this skill). Run this once, not on every
session — once the corpus is seeded, `agent-memory` is the loop to use.

## Procedure

1. Announce the repository scope the same way `agent-memory` does.
2. Index code: `comemory index-code --repo <scope> --path .` (or the
   published wrapper's `index-code` verb, which auto-injects `--repo`).
3. Index docs: `comemory index <dir> --repo <scope>` for each docs folder
   that actually exists in this repo (e.g. `docs/`, `README.md`) — skip the
   ones that don't.
4. When a platform session id is known, distill it:
   `comemory distill --session-id <id> --transcript <path> --dry-run` first,
   then without `--dry-run` to file the candidates.
5. Save the decisions the repo cannot derive from itself: why a choice was
   made, a constraint discovered the hard way, a convention the code doesn't
   spell out. Not copies of documentation, and not a restatement of what
   `index`/`index-code` already made searchable — those cover the *what*;
   save the *why*.

## Pitfalls

- Do not save summaries of files that indexing already covers — that's
  duplicate content with no reasoning attached.
- Do not skip `--repo` on raw `index-code`/`index`/`distill` calls; an
  unscoped bootstrap corpus is unusable by the repo-scoped recall hooks.
- Do not distill a transcript with no session id — the platform join key is
  required; there is no local-only distill.
- A single bootstrap pass does not replace the ongoing loop: keep saving
  verified corrections and decisions as they happen, per `agent-memory`.

## Verification

`comemory recall-status --repo <scope> --json` shows `saves > 0` for this
run's window, and `comemory list --repo <scope>` (or the wrapper's `list`)
shows the new entries.
