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
on stdin) flags exposed on `comemory save` and `comemory search`. The
caller-supplied vectors must match the dims baked into the vec0 DDL —
1024 for `memory_vec`, 768 for `code_vec` (defined in
`src/store/sql/0002_v2_tables.sql`); mismatched dims surface as
`VecDimMismatch`. To use a different embedder dim, edit the DDL literal
and rebuild. Tag the embedder you used via `COMEMORY_EMBED_HINT` so
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
| v0.2    | ~8 MB          | one SQLite file, BYO vectors, trimmed tree-sitter set |

## Install

### Homebrew (macOS + Linuxbrew)

```bash
brew install Falconiere/tap/comemory
```

### Curl installer

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/Falconiere/comemory/releases/latest/download/comemory-installer.sh \
  | sh
```

Drops the binary in `$CARGO_HOME/bin` (or `~/.local/bin`) and installs
shell completions.

### From source (contributors)

```bash
git clone https://github.com/Falconiere/comemory && cd comemory
bash scripts/dev-install.sh
```

Builds and installs to `$CARGO_HOME/bin` via `cargo install --path .`.

Prebuilt binaries for macOS (aarch64, x86_64) and Linux (aarch64, x86_64)
are published on the
[GitHub Releases](https://github.com/Falconiere/comemory/releases) page.

After install, run `comemory doctor` to verify the SQLite store and
data directory.

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
| `comemory feedback` | Record per-memory feedback (used vs irrelevant) against a search's `query_id` |
| `comemory eval` | Score retrieval quality (recall@k, MRR) against a golden set (feedback-harvested and/or `--golden` YAML) |
| `comemory mine` | Distill failed→successful query rewordings into expansion mappings (`--apply` feeds search) |
| `comemory tune` | Grid-search the ranking knobs against the golden set (`--apply` writes the winner to `config.toml`) |
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

## Learning loop

Every `comemory search` and `comemory context` lookup is logged
automatically: the JSON envelope (and the TTY footer) carries a `query_id`.
Feed back which hits actually helped, then measure and tune retrieval
against that ground truth:

```bash
# 1. Search — note the query_id in the output
comemory search "postgres pool exhausted" --json
# {"hits":[...],"query_id":"q-20260611-a1b2c3d4"}

# 2. Record which hits you used (and which were noise)
comemory feedback q-20260611-a1b2c3d4 --used a1b2c3d4 --irrelevant 00112233

# 3. Score retrieval (recall@k + MRR) against feedback-harvested golden
#    pairs, optionally merged with a hand-written YAML file (file wins)
comemory eval
comemory eval --golden golden.yaml --golden-only --k 5 --json

# 4. Distill failed→successful query rewordings into expansions that the
#    lexical fallback ladder applies (mappings need support >= 2)
comemory mine --apply

# 5. Grid-search the ranking knobs; --apply rewrites config.toml only when
#    the winner strictly beats the current config (needs >= 10 golden
#    pairs; the rewrite drops any comments in config.toml)
comemory tune --apply
```

Feedback both reranks future searches (Beta-smoothed boost/demotion) and
doubles as eval ground truth. Raw telemetry (`retrieval_log`,
`feedback_events`) is swept by `comemory gc` after 90 days
(`COMEMORY_LEARNING_RETENTION_DAYS`); aggregated feedback counters and
mined expansions are distilled knowledge and never expire — `comemory
rebuild` carries all learning state across too. Note that eval replays
are lexical and unfiltered: any `--repo`/`--kind` filters on the
originating search are ignored, so pairs born under filters are scored
against the unfiltered candidate pool.

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
# or: bash scripts/dev-install.sh --with-tools
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
(`scripts/dev-install.sh` already uses this). Distributed binaries are built
with `[profile.dist]` (inherits `[profile.release]`, overrides `lto = "thin"`
for faster CI build times).
