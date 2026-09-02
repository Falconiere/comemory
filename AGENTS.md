# AGENTS.md

This project's full documentation lives in [`CLAUDE.md`](./CLAUDE.md) — project
overview, architecture, binding rules, code style, module map, environment
variables, data model, save flow, testing, quality gates, distribution, and
Claude Code hooks.

## Quick start

```bash
just check                      # umbrella quality gate (fmt, type-check, lint, all checks)
just test                       # cargo nextest run --all-features
just qa                         # check + cargo-deny + dup-check + machete
cargo install --path .          # build + install binary locally
```

## Conventions that are easy to misread

Two rules account for most false readings of this tree. Both are enforced
mechanically — `bash scripts/guardrails/run.sh` is the check, and CI runs it
on every push, so a violation is a red build rather than a judgement call.

- **`requireReadme` names FOLDERS, not files.** `src/<module>/README.md` is
  the per-file index for the files inside `src/<module>/`. A single-file
  module — `src/serve/scope.rs`, `src/store/memory_purge.rs`,
  `src/config/patch.rs` — has no folder of its own and needs no README of
  its own; it is listed in its parent folder's README, and its `//!` module
  doc is its documentation. `guardrails.config.json`'s `src.requireReadme`
  lists exactly the folders that must carry one.
- **A `POST` is not automatically "mutating".** `RouteEntry::mutating`
  drives one thing, the read-only gate: it is a claim about whether the
  route writes to the STORE. `POST /jobs/{id}/cancel` is `mutating: false`
  on purpose, so a runaway job can still be stopped on a `--read-only`
  server.

## Where to start reading

- `src/main.rs` / `src/lib.rs` — binary entry + library surface
- `src/cli.rs` — subcommand dispatcher
- `src/cli/save.rs` — the save flow (markdown → SQLite transaction)
- `src/retrieval/pipeline.rs` — search orchestration
- `src/store.rs` — SQLite connection management
- `docs/architecture.md` — full architecture explanation
