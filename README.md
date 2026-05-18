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

# Homebrew (prebuilt binary via the SidegigLLC tap)
brew install SidegigLLC/tap/qwick
```

Prebuilt binaries for macOS (aarch64, x86_64) and Linux (aarch64, x86_64)
are published on the
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

# Run an ast-grep pattern against a single file
qwick ast 'fn $NAME($$$) { $$$ }' --file src/lib.rs

# Health check
qwick doctor
```

## Full command surface

| Command | Purpose |
|---------|---------|
| `qwick save` | Save a memory (body via arg, `-`, or stdin) |
| `qwick search` | Search the memory index by natural-language query |
| `qwick list` | List memories with optional repo/kind filters |
| `qwick delete` | Soft-delete a memory by id (moves to `.trash/`) |
| `qwick feedback` | Record per-memory feedback (used vs irrelevant) |
| `qwick doctor` | Report on the data directory and memory count |
| `qwick index-code` | Walk a repo, extract symbols, upsert into the code index |
| `qwick symbol` | Semantic search over the code index for a symbol name |
| `qwick memory-for` | List memories that reference a qualified symbol or file path |
| `qwick ast` | Run an ast-grep pattern against a single source file |
| `qwick context` | Headline lookup: code symbol + memories matching a key |
| `qwick walk` | Walk a graph edge from a memory id (e.g. `--edge supersedes`) |
| `qwick conflicts` | List memories that conflict with the given memory id |
| `qwick supersedes` | Record that one memory supersedes another in the kuzu graph |
| `qwick prune` | Detect (and optionally soft-delete) stale memories |
| `qwick gc` | Purge old entries from `memories/.trash/` |
| `qwick install-hooks` | Install git hooks that run `qwick index-code --incremental` on `post-commit`, `post-merge`, `post-checkout` |

All commands accept `--json` for machine-readable output. Exit codes follow
`sysexits.h` conventions. The data root defaults to `$HOME/.qwick` and can be
overridden with `--data-dir` or the `QWICK_DATA_DIR` environment variable.

## Quality Gates

The umbrella gate is `bash scripts/check-all.sh`, which runs `fmt-check`,
`type-check`, `lint-check`, `test-placement-check`, `no-bypass-check`,
`module-size-check`, `tests-mirror-check`, and `typos-check` in order. Use
`just check`, `just test`, or `just qa` for everyday workflows; CI runs the
same scripts so local + CI parity is one command away.

## Contributing

Read [CLAUDE.md](CLAUDE.md) first — it documents the architecture, the five
binding rules every contribution must satisfy (no duplication, modular
modules, ≤500 lines per file, zero warnings, tests strictly in `tests/`
mirroring `src/`), the module map, the frontmatter schema, and the
`.claude/hooks/` integration.

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

- `qwick save` writes the markdown + frontmatter and upserts the `Memory`
  node + `InRepo` / `AuthoredBy` / `Tagged` / `ReferencesFile` /
  `ReferencesSymbol` edges into kuzu, but does **not** yet embed the body
  into `lancedb.memory_chunks` from the save path itself — rebuild via
  `qwick index-code` for now. `RelatesTo` neighbor discovery is also
  deferred.
- `stale_code::detect` is a stub that returns an empty list. v1.1 will walk
  `references.files` for each memory against the repo's tracked files and
  flag mismatches as stale.
- LLM-driven supersedes / conflicts detection is out of scope. The current
  implementation only records explicit edges via the `supersedes` and graph
  commands.

## License

MIT
