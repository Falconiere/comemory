# Review, Fix & Commit Completed Work

Run AFTER all tasks are completed. Reviews changes on the current branch, fixes gaps, and commits. Does NOT merge. Does NOT push.

## 1. Identify scope

For each **git repo** with changes (see `/commit` for resolving targets from CWD, manifest aliases, or `./qw repo status`):

- Current branch: `git rev-parse --abbrev-ref HEAD`.
- **Base ref** — try in order until one exists: `origin/development`, `origin/main`, `development`, `main` (`git rev-parse --verify <ref>`).
- Committed changes: `git log <base-ref>..HEAD --oneline` + `git diff <base-ref>...HEAD --stat`.
- Uncommitted changes: `git status --short` + `git diff --stat HEAD`.
- If no changes exist (committed or working-tree), skip that repo.
- If nothing to review anywhere, STOP and report "nothing to review".
- Group changed files by repo (one repo may contain multiple packages — keep package boundaries when dispatching review).

## 2. Launch parallel review subagents

For each repo (or package within a repo) with changes, dispatch a subagent (use `superpowers:dispatching-parallel-agents` skill) that:

- Reads every changed file in its scope — both committed diff and working-tree state.
- Searches for gaps: missing error handling at boundaries, untested paths, dead code, over-engineering, unclear naming.
- Searches for simplification: unnecessary abstractions, duplicated logic, verbose code.
- Verifies tests exist for new/changed behavior and use real data (NO mocks).
- Reports concrete issues with file paths and line numbers — no vague feedback accepted.

## 3. Fix all reported issues

- Fix every gap, simplification, and missing test reported by the subagents.
- Fix any pre-existing errors or warnings in touched files (zero tolerance).
- Same approach failed twice? STOP — change hypothesis, don't retry harder.

## 4. Run quality gates

Per repo with fixes, run the checks documented in that repo's `CLAUDE.md` / `AGENTS.md` (and `scripts/quality-gates/` when applicable) — **ZERO errors, ZERO warnings**.

If any gate fails, fix and re-run until green. No exceptions.

## 5. Commit

Follow `/commit` for each repo with changes:

- Stage intentionally: only files that belong in this commit.
- Use that repo's commit conventions (not a single global prefix list unless the repo defines one).
- One logical change per commit — split when scope requires it.
- Never use `--no-verify`. If hooks fail, fix the underlying issue and retry.

## 6. Completion

- Report: what was reviewed, what was fixed, final gate status, commit hashes produced (per repo).
- Do not mark complete unless all gates are green AND each affected repo's working tree is clean.
