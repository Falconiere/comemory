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

## Where to start reading

- `src/main.rs` / `src/lib.rs` — binary entry + library surface
- `src/cli/mod.rs` — subcommand dispatcher
- `src/cli/save.rs` — the save flow (markdown → SQLite transaction)
- `src/retrieval/pipeline.rs` — search orchestration
- `src/store/mod.rs` — SQLite connection management
- `docs/architecture.md` — full architecture explanation
