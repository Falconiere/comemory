# comemory

Agentic dev memory + code-aware semantic search via a two-layer property graph.

`comemory` is a single Rust binary that captures developer knowledge (decisions,
bugs, conventions, discoveries) as markdown files and links it to your code
through one SQLite file backed by FTS5 and `sqlite-vec`. Everything runs
locally — no API calls, no remote database, no in-process LLM.

## BYO-Vector workflow

`comemory` v0.2 ships **without** a bundled embedding model. Lexical search
works out of the box using SQLite FTS5 — `comemory save` and `comemory search`
do not require any embedder to be configured. Run them as-is to get full-text
results immediately after install.

For dense / semantic retrieval, *you* supply the vectors. Pipe them in via
the `--vector` (CSV) or `--vector-stdin` (JSON `{"embedding":[..]}` payload
on stdin) flags exposed on `comemory save` and `comemory search`. The first
write locks the dimensionality into `schema_meta`; later inserts that do
not match surface as `VecDimMismatch`. Tune the configured dim with
`COMEMORY_VECTOR_DIM` (memories) and `COMEMORY_CODE_VECTOR_DIM` (code
symbols). Tag the embedder you used via `COMEMORY_EMBED_HINT` so
`comemory doctor` can surface it.

A wrapper script that bridges to a local Ollama instance ships in
`scripts/comemory-embed.sh`:

```bash
# Save a decision and embed the body via Ollama in one call
scripts/comemory-embed.sh save "Use Postgres for analytics" \
  --kind decision --repo myrepo

# Semantic search routed through the same Ollama model
scripts/comemory-embed.sh search "what database do we use"
```

The wrapper is documentation, not enforcement — replace it with whatever
embedder pipeline (OpenAI, Voyage, a local llama.cpp, …) you prefer.

### Binary size

The lightweight refactor cuts the release binary substantially by dropping
the in-process embedder, vector database, and graph database:

| Version | Release binary | Notes |
|---------|---------------:|-------|
| v0.1    | ~117 MB        | bundled fastembed + lancedb + kuzu |
| v0.2    | ~25 MB         | one SQLite file, BYO vectors        |

## Install

```bash
# From source (Cargo)
cargo install comemory

# Homebrew (prebuilt binary via the Falconiere tap)
brew install Falconiere/tap/comemory
```

Prebuilt binaries for macOS (aarch64, x86_64) and Linux (aarch64, x86_64)
are published on the
[GitHub Releases](https://github.com/Falconiere/comemory/releases) page.

## 60-second tour

```bash
# Save a memory (auto-detects repo + author from git)
comemory save "Use Postgres for analytics" --kind decision --repo myrepo --tags db,postgres

# Index your repo's code (symbols + files into the SQLite store)
comemory index-code --repo myrepo --path .

# One-shot bundle for a symbol: source, memories, neighborhood
comemory context run_migration --json

# Search across memories (lexical-only, no embedder required)
comemory search "what database do we use"

# Semantic search with a caller-supplied vector
echo '{"embedding":[0.1,0.2,...]}' \
  | comemory search "what database do we use" --vector-stdin

# Run an ast-grep pattern against a single file
comemory ast 'fn $NAME($$$) { $$$ }' --lang rs --file src/lib.rs

# Health check
comemory doctor
```

## Full command surface

| Command | Purpose |
|---------|---------|
| `comemory save` | Save a memory (body via arg, `-`, or stdin; optional `--vector` / `--vector-stdin`) |
| `comemory search` | Search the memory index by natural-language query (lexical by default, hybrid when `--vector` / `--vector-stdin` is supplied) |
| `comemory list` | List memories with optional repo/kind filters |
| `comemory delete` | Soft-delete a memory by id (moves to `.trash/`) |
| `comemory feedback` | Record per-memory feedback (used vs irrelevant) |
| `comemory doctor` | Report on the data directory and SQLite mirror health |
| `comemory index-code` | Walk a repo, extract symbols, upsert into the code index |
| `comemory ingest-code` | Read pre-embedded JSONL from stdin into the code index |
| `comemory ast` | Run an ast-grep pattern against a single source file |
| `comemory context` | Headline lookup: code symbol + memories matching a key |
| `comemory prune` | Detect (and optionally soft-delete) stale memories |
| `comemory rebuild` | Drop `comemory.db` and repopulate it from `memories/*.md` |
| `comemory gc` | Purge old entries from `memories/.trash/` |
| `comemory install-hooks` | Install git hooks that run `comemory index-code --incremental` on `post-commit`, `post-merge`, `post-checkout` |

All commands accept `--json` for machine-readable output. Exit codes follow
`sysexits.h` conventions. The data root defaults to `$HOME/.comemory` and can be
overridden with `--data-dir` or the `COMEMORY_DATA_DIR` environment variable.

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

### Benchmarks

`just bench` runs the criterion harness and writes `docs/bench/latest.md`.
The save and retrieval suites cover the embed-on-save path and the RRF-fused
dense+BM25 search introduced in v1.1.

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
