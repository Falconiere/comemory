# qwick-memory

Agentic dev memory + code-aware semantic search via a two-layer property graph.

`qwick-memory` is a single Rust binary that captures developer knowledge (decisions,
bugs, conventions, discoveries) as markdown files and links it to your code
through a kuzu property graph and LanceDB vector indices. Everything runs
locally — no API calls, no remote database, no in-process LLM.

## Install

```bash
# From source (Cargo)
cargo install qwick-memory

# Homebrew (prebuilt binary via the SidegigLLC tap)
brew install SidegigLLC/tap/qwick-memory
```

Prebuilt binaries for macOS (aarch64, x86_64) and Linux (aarch64, x86_64)
are published on the
[GitHub Releases](https://github.com/SidegigLLC/qwick-memory/releases) page.

## 60-second tour

```bash
# Save a memory (auto-detects repo + author from git)
qwick-memory save "Use Postgres for analytics" --kind decision --repo myrepo --tags db,postgres

# Index your repo's code (symbols + files into kuzu + LanceDB)
qwick-memory index-code --root . --repo myrepo

# One-shot bundle for a symbol: source, memories, neighborhood
qwick-memory context run_migration --json

# Search across memories
qwick-memory search "what database do we use" --limit 5

# Semantic search over code symbols
qwick-memory symbol parse_frontmatter

# Memories that reference a file or symbol
qwick-memory memory-for myrepo:src/db.rs:run_migration

# Run an ast-grep pattern against a single file
qwick-memory ast 'fn $NAME($$$) { $$$ }' --file src/lib.rs

# Health check
qwick-memory doctor
```

## Graph viewer

`qwick-memory graph serve` opens a local browser-based viewer for the
property graph. Click-to-expand neighbours, search across kinds, filter
by node kind, render memory bodies inline. Loopback-only; assets are
embedded in the binary.

```bash
# Open the viewer in the default browser
qwick-memory graph serve

# Headless / over SSH
qwick-memory graph serve --no-open

# Pin a port
qwick-memory graph serve --port 7878
```

See [docs/graph-viewer.md](docs/graph-viewer.md) for the REST API, smoke
checklist, and architecture notes.

## Full command surface

| Command | Purpose |
|---------|---------|
| `qwick-memory save` | Save a memory (body via arg, `-`, or stdin) |
| `qwick-memory search` | Search the memory index by natural-language query |
| `qwick-memory list` | List memories with optional repo/kind filters |
| `qwick-memory delete` | Soft-delete a memory by id (moves to `.trash/`) |
| `qwick-memory feedback` | Record per-memory feedback (used vs irrelevant) |
| `qwick-memory doctor` | Report on the data directory and memory count |
| `qwick-memory index-code` | Walk a repo, extract symbols, upsert into the code index |
| `qwick-memory symbol` | Semantic search over the code index for a symbol name |
| `qwick-memory memory-for` | List memories that reference a qualified symbol or file path |
| `qwick-memory ast` | Run an ast-grep pattern against a single source file |
| `qwick-memory context` | Headline lookup: code symbol + memories matching a key |
| `qwick-memory walk` | Walk a graph edge from a memory id (e.g. `--edge supersedes`) |
| `qwick-memory conflicts` | List memories that conflict with the given memory id |
| `qwick-memory supersedes` | Record that one memory supersedes another in the kuzu graph |
| `qwick-memory prune` | Detect (and optionally soft-delete) stale memories |
| `qwick-memory gc` | Purge old entries from `memories/.trash/` |
| `qwick-memory install-hooks` | Install git hooks that run `qwick-memory index-code --incremental` on `post-commit`, `post-merge`, `post-checkout` |
| `qwick-memory graph serve` | Open a local browser-based viewer for the property graph |

All commands accept `--json` for machine-readable output. Exit codes follow
`sysexits.h` conventions. The data root defaults to `$HOME/.qwick-memory` and can be
overridden with `--data-dir` or the `QWICK_MEMORY_DATA_DIR` environment variable.

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
- [Graph viewer](docs/graph-viewer.md) — REST endpoints, smoke checklist,
  and architecture notes for `qwick-memory graph serve`.
- [Design spec](docs/superpowers/specs/2026-05-17-qwick-rust-agentic-rag-design.md) —
  full specification, including the data model, kuzu schema, and risk register.
- [Implementation plan](docs/superpowers/plans/2026-05-17-qwick-rust-agentic-rag-plan.md) —
  the 22-task TDD plan this codebase was built from.

## Known v1.1 gaps

`qwick-memory` v1.0 ships the full retrieval pipeline, kuzu graph, and code indexer.
The following items are intentionally deferred to v1.1:

- `qwick-memory save` writes the markdown + frontmatter and upserts the `Memory`
  node + `InRepo` / `AuthoredBy` / `Tagged` / `ReferencesFile` /
  `ReferencesSymbol` edges into kuzu, but does **not** yet embed the body
  into `lancedb.memory_chunks` from the save path itself — rebuild via
  `qwick-memory index-code` for now. `RelatesTo` neighbor discovery is also
  deferred.
- `stale_code::detect` is a stub that returns an empty list. v1.1 will walk
  `references.files` for each memory against the repo's tracked files and
  flag mismatches as stale.
- LLM-driven supersedes / conflicts detection is out of scope. The current
  implementation only records explicit edges via the `supersedes` and graph
  commands.

## License

MIT

## Optional: faster builds

These tools are entirely optional. The project builds with stock
`cargo` out of the box.

- **sccache** — caches rustc outputs across `cargo clean`. ~5–20%
  cold-build win on a warm cache.
- **hyperfine** — used by `just perf` to measure warm-incremental p50/p95.

Install (Apple Silicon):

```bash
brew install sccache hyperfine
# or: bash scripts/install.sh --with-tools
```

Activate sccache by exporting in your shell init:

```bash
export RUSTC_WRAPPER=sccache
```

Measure your builds:

```bash
just perf            # writes .build-perf/summary.json
bash scripts/build-perf.sh --append-md   # also appends a row to docs/build-perf.md
```

Local fast release builds: `cargo build --profile release-quick`
(`scripts/install.sh` already uses this). Distributed binaries continue
to use `[profile.release]` via `cargo-dist`.
