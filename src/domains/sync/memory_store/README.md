# api/memory_store/

**What belongs here:** the `git` subprocess steps the `store-sync` job runs
(`POST /api/v1/memory-stores/{id}/sync`) — split out of
`src/api/memory_store.rs` once the two-shape conflict handling (a rebase
stopped on `CONFLICT`, and an autostash re-apply that conflicts while git
exits zero) pushed that file past the 300-line ceiling.

**What does NOT belong here:** the store's view (`Store`, `SyncState`), the
`[git]` config patch, the `501` create refusal, the step ORDER of a sync
(commit → pull → push, and why), and the in-process `git2` read probes
(`sync_state`, `upstream_target`, `dirty_count`, `work_tree`) — those stay
in `src/api/memory_store.rs`, since the probes must be cheap enough to run
on every console poll and never shell out.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `git.rs` | `run` | Every `git` shell-out of the sync job through one captured, prompt-hardened `run` (stdin `/dev/null`, `GIT_TERMINAL_PROMPT=0`); `pull` (rebase + autostash, both conflict shapes reported by path, the stopped rebase aborted), `commit_memories` (the pathspec-limited `git add -A` + `git commit -- <memories>`), `head`, and `push` (`git push <remote> HEAD` when `[git] remote` is set, else a bare `git push`) |
