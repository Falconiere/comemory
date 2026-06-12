<div align="center">

# 🧠 comemory

### Agentic developer memory + code-aware semantic search — in a single Rust binary.

Capture the *why* behind your code as markdown, link it to the *what* in your
source, and get it back through hybrid search that actually understands
identifiers, decay, and your git graph. **100% local. No API keys. No daemon.
No in-process LLM.**

[![Release](https://img.shields.io/github/v/release/Falconiere/comemory?style=flat-square&color=blue)](https://github.com/Falconiere/comemory/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-green.svg?style=flat-square)](LICENSE)
[![Rust 1.95+](https://img.shields.io/badge/rust-1.95%2B-orange.svg?style=flat-square)](https://www.rust-lang.org)
[![Single binary](https://img.shields.io/badge/runtime-single%20binary-purple.svg?style=flat-square)](#install)
[![Local-first](https://img.shields.io/badge/privacy-100%25%20local-brightgreen.svg?style=flat-square)](#why-comemory)

[Why](#why-comemory) · [Features](#features) · [Install](#install) · [Quickstart](#quickstart) · [Concepts](#core-concepts) · [Commands](#command-reference) · [Architecture](docs/architecture.md)

</div>

---

## Why comemory?

Every codebase carries two kinds of knowledge:

- **The code** — what the system *does*, readable straight from source.
- **The memory** — *why* it does it that way: the decision you made at 2 a.m.,
  the bug that bit you twice, the convention nobody wrote down, the discovery
  that saved a week.

The second kind evaporates. It lives in Slack threads, closed PRs, and the head
of whoever left last quarter. Meanwhile your tools only search the first kind —
and they do it with plain substring or keyword match that can't tell
`runMigration` from `run_migration`, can't tell a hot file from a dead one, and
has no idea which symbols change *together*.

**comemory fuses both layers into one local property graph.** You save short
markdown memories; it extracts symbols from your repo; it mines your git history
for which files co-change and which import which; it ranks everything with a
deterministic blend of full-text relevance, ACT-R memory decay, PageRank graph
centrality, and your own feedback. Markdown stays the source of truth — one
SQLite file is the rebuildable index.

> No embeddings server to run, no vector DB to host, no LLM round-trips. It's a
> ~10 MB binary you drop on your PATH. Bring your own embedder *if* you want
> dense search; lexical works out of the box.

---

## Features

| | |
|---|---|
| 🗒️ **Memory as markdown** | Decisions, bugs, conventions, discoveries — plain `.md` files with YAML frontmatter at `~/.comemory/memories/`. Git-friendly, human-editable, the single source of truth. |
| 🔎 **Hybrid retrieval** | FTS5 BM25 + optional BYO-vector ANN, fused via Reciprocal Rank Fusion, with a 4-tier lexical fallback ladder ending in *mined* query expansions. |
| 🧬 **Identifier-aware search** | A custom FTS5 tokenizer splits `camelCase` / `snake_case`, so `parseFrontmatter` matches `parse_frontmatter` matches `frontmatter parsing`. |
| 🕸️ **Two-layer code graph** | `index-code` mines **co-change** edges from git history and **import** edges per language, then materializes a weighted **PageRank** onto every symbol. |
| 🧠 **Memory that decays** | ACT-R activation (recency × access count) and Beta-smoothed feedback rerank results the way human memory actually surfaces things. |
| 📈 **A real learning loop** | Record which hits helped → score recall@k / MRR against a golden set → mine reformulations → grid-search the ranking knobs. All offline, all deterministic. |
| 🌐 **Interactive web viewer** | `comemory serve` ships a loopback-only React SPA (embedded in the binary) to explore the WebGL code graph and *edit source in the browser* — no Node toolchain at runtime. |
| 🌳 **AST patterns** | `comemory ast` runs ast-grep structural patterns over Rust, TypeScript, JavaScript, Python, and Go. |
| 🔌 **Machine-friendly** | `--json` on every command, `score_parts` explainability contract, exit codes per `sysexits.h`. |
| 📦 **One binary, fully local** | One SQLite file backs FTS5 + `sqlite-vec` + edges. Rebuildable from markdown at any time with `comemory rebuild`. |

---

## How it works

comemory is a **two-layer property graph** stitched together by typed edges in
one SQLite file:

```
        MEMORY LAYER                              CODE LAYER
   (markdown, source of truth)            (extracted from your repo)

   ┌────────────────────┐                 ┌────────────────────┐
   │  decision  a1b2c3d4 │  references     │  run_migration      │
   │  "use Postgres for  │ ──────────────▶ │  src/db.rs          │
   │   analytics"        │                 │  rank_score: 0.82   │
   └─────────┬──────────┘                 └─────────┬──────────┘
             │ supersedes                            │ co_changed (git)
             ▼                                       ▼  imports (lang)
   ┌────────────────────┐                 ┌────────────────────┐
   │  decision  9f8e7d6c │                 │  apply_pool_config  │
   │  (older, demoted)   │                 │  src/pool.rs        │
   └────────────────────┘                 └────────────────────┘

   ─────────────────────────────────────────────────────────────
              one SQLite file:  comemory.db
   memories · memory_fts · memory_vec · code_symbols · code_fts
   code_vec · edges · learning-loop telemetry
```

A query runs through a pure-Rust pipeline — **route** (candidates + lexical
ladder) → **rerank** (multiplicative priors over relevance) → **diversify**
(SimHash near-dup collapse + MMR) → **cited bundle**. No LLM calls anywhere.
See [`docs/architecture.md`](docs/architecture.md) for the full diagram.

---

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

Drops the binary in `$CARGO_HOME/bin` (or `~/.local/bin`) and installs shell
completions.

### From source

```bash
git clone https://github.com/Falconiere/comemory && cd comemory
cargo install --path .          # or: bash scripts/dev-install.sh
```

Prebuilt binaries for **macOS** (aarch64, x86_64) and **Linux** (aarch64,
x86_64) are attached to every
[GitHub Release](https://github.com/Falconiere/comemory/releases).

After install, run `comemory doctor` to verify the SQLite store and data
directory.

<details>
<summary><b>Binary size over time</b></summary>

| Version | Release binary | Notes |
|---------|---------------:|-------|
| v0.1    | ~117 MB        | bundled fastembed + lancedb + kuzu |
| v0.2    | ~8 MB          | one SQLite file, BYO vectors, trimmed tree-sitter set |
| v0.7    | ~10.5 MB       | adds the `serve` web SPA, embedded + gzip-compressed |

The v0.2 rewrite dropped the in-process embedder, vector DB, and graph DB. The
web viewer added since is the only meaningful weight back, and it's
gzip-compressed in the binary.

</details>

---

## Quickstart

```bash
# 1. Save a memory — repo + author auto-detected from git
comemory save "Use Postgres for analytics, not ClickHouse — see ADR-14" \
  --kind decision --repo myrepo --tags db,postgres

# 2. Index your code — symbols + co-change/import graph + PageRank
comemory index-code --repo myrepo --path .

# 3. Search memories (lexical, no embedder needed)
comemory search "what database do we use"

# 4. Ranked code search — BM25 + graph priors; --json carries score_parts
comemory search-code "connection pool retry" --repo myrepo

# 5. One-shot bundle for a symbol: source + related memories + neighborhood
comemory context run_migration --json

# 6. Explore it all in the browser (loopback-only, opens a token URL)
comemory serve --open

# 7. Health check
comemory doctor
```

That's the whole loop: **capture → index → recall**. Dense/semantic search is
opt-in (see [BYO-Vector](#byo-vector-workflow)); everything above works with
zero configuration.

---

## Core Concepts

### Memory: capture & recall

A memory is a markdown file with frontmatter — schema v1:

```yaml
---
id: a1b2c3d4                  # 8-hex prefix of SHA-256(body.trim_end())
kind: decision                # decision | bug | convention | discovery | pattern | note
repo: myrepo
tags: [database, postgres]
author: falconiere
created: 2026-05-17T14:30:00Z
quality: 4                    # 1–5, biases ranking
schema: 1
content_hash: <64-hex SHA-256 of body.trim_end()>
references:                   # indexer-managed links into the code layer
  symbols: [myrepo:src/db.rs:run_migration]
  files:   [myrepo:src/db.rs]
relations:                    # memory→memory edges
  supersedes:     []
  conflicts_with: []
  derived_from:   []
---

Postgres handles our analytics volume fine with proper indexing.
ClickHouse added ops burden we didn't need. See `myrepo:src/db.rs`.
```

Backticked `<repo>:<path>` / `<repo>:<path>:<symbol>` mentions in the body are
auto-extracted into edges, so memories stitch themselves to code. On save,
comemory runs a **near-duplicate check** (64-bit SimHash) and, if you re-save a
refined version, `--supersedes <id>` demotes the old one in every future
ranking.

### Code search: BM25 + a mined graph

`comemory index-code` does far more than list symbols. It:

1. Extracts symbols via ast-grep (Rust / TS / JS / Python / Go), splitting
   oversized ones into AST-boundary child chunks (cAST).
2. Mines **co-change** edges from your git history (which files move together).
3. Resolves per-language **import** edges.
4. Runs weighted **PageRank** over that graph and writes `rank_score` onto every
   symbol.

`comemory search-code` then fuses weighted BM25 (over identifiers, snippets, and
path tokens) with an optional BYO-vector ANN leg, and reranks by **four priors**:
PageRank centrality, recency, working-set affinity (dirty/recently-touched files
near a hit, when you search inside the checkout), and feedback. Every hit carries
a `score_parts` breakdown explaining exactly why it ranked where it did.

### The learning loop

Every `search` and `context` lookup emits a `query_id`. Close the loop:

```bash
# 1. Search — note the query_id
comemory search "postgres pool exhausted" --json
#    {"hits":[...],"query_id":"q-20260611-a1b2c3d4"}

# 2. Tell it what actually helped (and what was noise)
comemory feedback q-20260611-a1b2c3d4 --used a1b2c3d4 --irrelevant 00112233

# 3. Score retrieval quality against feedback-harvested + YAML golden pairs
comemory eval --k 5 --json

# 4. Distill failed→successful rewordings into expansions the search applies
comemory mine --apply

# 5. Grid-search the ranking knobs; --apply rewrites config.toml only on a win
comemory tune --apply
```

Feedback does double duty: it reranks future searches (Beta-smoothed) *and* it's
eval ground truth. Raw telemetry is swept after 90 days; distilled knowledge
(aggregated counters, mined expansions) never expires and survives
`comemory rebuild`.

### Interactive web viewer

```bash
comemory serve --open                 # ephemeral port, prints a token URL
comemory serve --port 8787 --read-only  # pin a port, disable writes
```

`comemory serve` boots an axum server **bound to `127.0.0.1` only**, handing out
a React/Vite/Tailwind SPA embedded in the binary. Explore the WebGL-rendered
(sigma.js + ForceAtlas2) code graph — pan/zoom/hover stay smooth into ~100k
nodes — and open any indexed file in a CodeMirror 6 editor to view *and save*
edits, with `If-Match` optimistic concurrency keyed on the git blob OID.

> **Security posture:** loopback-only bind, a 256-bit per-session token
> (constant-time compared, set as an `HttpOnly; SameSite=Strict` cookie and
> stripped from the URL after first load), a `Host`-header guard against
> DNS-rebinding, default-deny CORS, an editable-extension allowlist, a 5 MiB
> write cap, and a single canonicalize-and-contain chokepoint that rejects
> `..` / absolute / symlink path escapes.

### AST patterns

```bash
comemory ast 'fn $NAME($$$) { $$$ }' --lang rs --file src/lib.rs
```

Structural search over Rust, TypeScript, JavaScript, Python, and Go via
ast-grep — find shapes, not strings.

---

## Command Reference

| Command | Purpose |
|---------|---------|
| `comemory save` | Save a memory (body via arg, `-`, or stdin; optional `--vector` / `--vector-stdin`) |
| `comemory search` | Search memories — lexical by default, hybrid when a vector is supplied |
| `comemory search-code` | Search the code index (BM25 + optional ANN, reranked by graph priors) |
| `comemory context` | One-shot bundle for a key: code symbol + related memories + neighborhood |
| `comemory list` | List memories with optional repo/kind filters |
| `comemory delete` | Soft-delete a memory by id (moves to `.trash/`) |
| `comemory feedback` | Record per-hit feedback against a `query_id` (`--used` / `--used-code` …) |
| `comemory eval` | Score retrieval quality (recall@k, MRR) against a golden set |
| `comemory mine` | Distill failed→successful query rewordings into expansions (`--apply`) |
| `comemory tune` | Grid-search ranking knobs against the golden set (`--apply` writes `config.toml`) |
| `comemory index-code` | Walk a repo, extract symbols, mine the co-change/import graph, run PageRank |
| `comemory ingest-code` | Read pre-embedded JSONL from stdin into the code index |
| `comemory graph` | Export the code-connection graph as JSON, Graphviz DOT, or interactive HTML |
| `comemory serve` | Loopback web viewer + in-browser editor over `comemory.db` and source |
| `comemory ast` | Run an ast-grep structural pattern against a source file |
| `comemory doctor` | Report on data-directory and SQLite-mirror health |
| `comemory prune` | Detect (and optionally soft-delete) stale memories |
| `comemory rebuild` | Drop `comemory.db` and repopulate it from `memories/*.md` |
| `comemory gc` | Purge old entries from `memories/.trash/` and aged telemetry |
| `comemory completions` | Generate shell completions |
| `comemory install-hooks` | Install git hooks that incrementally reindex on commit/merge/checkout |

Every command accepts `--json`. The data root defaults to `~/.comemory` and is
overridable with `--data-dir` or `COMEMORY_DATA_DIR`. Full per-command docs with
worked examples: **[docs/cli-reference.md](docs/cli-reference.md)**.

---

## Configuration

Config is layered: built-in defaults → `~/.comemory/config.toml` → environment →
CLI flags. A few of the most useful knobs:

| Variable | Purpose | Default |
|----------|---------|---------|
| `COMEMORY_DATA_DIR` | Root data directory | `~/.comemory` |
| `COMEMORY_RETRIEVAL_TOP_K` | Results returned by the hybrid router | `12` |
| `COMEMORY_INDEXING_AUTO_REINDEX` | `lazy` \| `hook` \| `off` auto code-index refresh | `lazy` |
| `COMEMORY_RANK_DECAY` | ACT-R decay exponent — higher = older memories fade faster | `0.5` |
| `COMEMORY_RANK_MMR_LAMBDA` | MMR relevance-vs-diversity trade-off `[0,1]` | `0.7` |
| `COMEMORY_GIT_AUTO_SYNC` | Best-effort commit + push after a save | `false` |
| `COMEMORY_EMBED_HINT` | Records the embedder you used (surfaced by `doctor`) | unset |

The complete table — including ranking, pruning, and BM25-weight knobs — lives in
[CLAUDE.md](CLAUDE.md#environment-variables).

---

## BYO-Vector workflow

comemory ships **without** a bundled embedding model. Lexical search works
immediately. For dense/semantic retrieval, *you* supply the vectors via
`--vector` (CSV) or `--vector-stdin` (JSON `{"embedding":[..]}`):

```bash
# Save + embed via a local Ollama model in one call (sample wrapper)
scripts/comemory-embed.sh save "Use Postgres for analytics" \
  --kind decision --repo myrepo

# Semantic search routed through the same model
scripts/comemory-embed.sh search "what database do we use"
```

Vectors must match the dims baked into the `vec0` DDL — **1024** for
`memory_vec`, **768** for `code_vec`. Mismatches fail fast with
`VecDimMismatch` rather than corrupting the index. The wrapper in
[`scripts/comemory-embed.sh`](scripts/comemory-embed.sh) is documentation, not
enforcement — swap in OpenAI, Voyage, llama.cpp, or anything else.

---

## Documentation

- **[Architecture overview](docs/architecture.md)** — storage, retrieval
  pipeline, save flow, and code-indexing flow on two pages.
- **[CLI reference](docs/cli-reference.md)** — every subcommand with arguments
  and worked examples.
- **[CHANGELOG](CHANGELOG.md)** — what changed, version by version.

---

## Contributing

Read **[CLAUDE.md](CLAUDE.md)** first — it documents the architecture, the
module map, the frontmatter schema, and the **five binding rules** every
contribution must satisfy:

1. No duplication — shared logic is extracted.
2. Very modular modules — narrow, single-purpose files.
3. ≤ 500 lines per file in `src/` or `scripts/`.
4. Zero errors, zero warnings — no `#[allow]`, no bare `.unwrap()`, no
   `println!` in `src/`.
5. Tests strictly in `tests/`, mirroring `src/` 1:1.

The umbrella quality gate is one command — CI runs the same scripts:

```bash
bash scripts/check-all.sh     # fmt · type · lint · placement · bypass · size · mirror · typos
just check                    # alias of the above
just test                     # cargo nextest run --all-features
just qa                       # check-all + cargo-deny + dup-check
just e2e                      # real-binary end-to-end harness
```

A task isn't done until `scripts/check-all.sh` exits 0.

---

## License

[MIT](LICENSE) © Falconiere Barbosa

<div align="center">
<sub>Built in Rust 🦀 · 100% local · one binary · markdown is the source of truth</sub>
</div>
