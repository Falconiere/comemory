# qwick

Agentic dev memory + code-aware semantic search via a two-layer property graph.

`qwick` is a single Rust binary that captures developer knowledge (decisions,
bugs, conventions, discoveries) as markdown files and links it to your code
through a kuzu property graph and LanceDB vector indices. Everything runs
locally — no API calls, no remote database, no in-process LLM.

## Install

```bash
# From source (Cargo)
cargo install qwick

# Homebrew (prebuilt binary)
brew install SidegigLLC/tap/qwick
```

Prebuilt binaries for macOS, Linux, and Windows are also published on the
[GitHub Releases](https://github.com/SidegigLLC/qwick/releases) page.

## 60-second tour

```bash
# Save a memory (auto-detects repo + author from git)
qwick save "Use Postgres for analytics" --kind decision --repo myrepo --tags db,postgres

# Index your repo's code (symbols + files into kuzu + LanceDB)
qwick index-code --root . --repo myrepo

# One-shot bundle for a symbol: source, memories, neighborhood
qwick context run_migration --json

# Search across memories
qwick search "what database do we use" --limit 5

# Semantic search over code symbols
qwick symbol parse_frontmatter

# Memories that reference a file or symbol
qwick memory-for myrepo:src/db.rs:run_migration

# Health check
qwick doctor
```

All commands accept `--json` for machine-readable output. Exit codes follow
`sysexits.h` conventions. The data root defaults to `$HOME/.qwick` and can be
overridden with `--data-dir` or the `QWICK_DATA_DIR` environment variable.

## Docs

- [Architecture overview](docs/architecture.md) — 2-page on-ramp into the
  storage, retrieval pipeline, save flow, and code-indexing flow.
- [CLI reference](docs/cli-reference.md) — every subcommand with arguments
  and worked examples.
- [Design spec](docs/superpowers/specs/2026-05-17-qwick-rust-agentic-rag-design.md) —
  full specification, including the data model, kuzu schema, and risk register.
- [Implementation plan](docs/superpowers/plans/2026-05-17-qwick-rust-agentic-rag-plan.md) —
  the 22-task TDD plan this codebase was built from.

## Known v1.1 gaps

`qwick` v1.0 ships the full retrieval pipeline, kuzu graph, and code indexer.
The following items are intentionally deferred to v1.1:

- `qwick save` writes the markdown + frontmatter and upserts the memory into
  LanceDB, but does **not** yet wire the kuzu `Memory` node upsert or the
  `references` cross-link extraction. Those edges land today only through
  `qwick index-code` and explicit graph commands (`supersedes`, `conflicts`).
  Tracked for v1.1.
- `stale_code::detect` is a stub that returns an empty list. v1.1 will walk
  `references.files` for each memory against the repo's tracked files and
  flag mismatches as stale.
- LLM-driven supersedes / conflicts detection is out of scope. The current
  implementation only records explicit edges via the `supersedes` and graph
  commands.

## License

MIT
