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

[Why](#why-comemory) · [Features](#features) · [Install](#install) · [Quickstart](#quickstart) · [Commands](#command-reference) · [Docs](#documentation) · [Architecture](docs/architecture.md)

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
one SQLite file — a **memory layer** (markdown, source of truth) and a **code
layer** (symbols extracted from your repo), joined by `references`, `supersedes`,
`co_changed`, and `imports` edges:

```
memories · memory_fts · memory_vec · code_symbols · code_fts
code_vec · edges · learning-loop telemetry   →  one file: comemory.db
```

A query runs through a pure-Rust pipeline — **route** (candidates + lexical
ladder) → **rerank** (multiplicative priors over relevance) → **diversify**
(SimHash near-dup collapse + MMR) → **cited bundle**. No LLM calls anywhere.
See [`docs/architecture.md`](docs/architecture.md) for the full diagram, storage
layout, and edge graph.

---

## Install

```bash
# Homebrew (macOS + Linuxbrew)
brew install Falconiere/tap/comemory

# From a local checkout (not published to crates.io)
git clone https://github.com/Falconiere/comemory && cd comemory
cargo install --path .
```

Then verify: `comemory doctor`. Prebuilt binaries for **macOS** (aarch64) and
**Linux** (aarch64, x86_64) are attached to every
[GitHub Release](https://github.com/Falconiere/comemory/releases).

Full install details — the curl installer and shell completions — are in
**[docs/getting-started.md](docs/getting-started.md)**; the binary-size history
lives in **[docs/build-perf.md](docs/build-perf.md)**.

---

## Quickstart

```bash
comemory save "Use Postgres for analytics, not ClickHouse — see ADR-14" \
  --kind decision --repo myrepo --tags db,postgres   # capture a memory
comemory index-code --repo myrepo --path .           # symbols + graph + PageRank
comemory search "what database do we use"            # recall memories (lexical)
comemory search-code "connection pool retry" --repo myrepo   # ranked code search
comemory context run_migration --json                # source + memories + neighbors
comemory serve --open                                # explore it in the browser
```

That's the whole loop: **capture → index → recall** — zero configuration. Dense/
semantic search is opt-in (see [BYO-Vector](#byo-vector-workflow)).

Full walkthrough — sandbox tips, JSON pagination, scoping flags:
**[docs/getting-started.md](docs/getting-started.md)**.

---

## Core concepts

A **memory** is a markdown file with YAML frontmatter (`id`, `kind`, `repo`,
`tags`, `quality`, plus `references` into code and `relations` between memories).
Backticked `<repo>:<path>:<symbol>` mentions in the body auto-link to the code
layer; a SimHash near-dup check and `--supersedes` keep the store tidy.
**Code search** blends weighted BM25 over identifiers/snippets/paths with an
optional BYO-vector ANN leg, reranked by four graph priors (PageRank, recency,
working-set affinity, feedback), every hit carrying a `score_parts` breakdown.
A deterministic **learning loop** (`feedback → eval → mine → tune`) measures and
improves ranking offline.

Full data model, save flow, retrieval pipeline, and graph mechanics:
**[docs/architecture.md](docs/architecture.md)**.

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
| `comemory install-hooks` | Install git hooks that reindex code on commit/merge/checkout |

Every command accepts `--json`; the data root defaults to `~/.comemory`
(overridable with `--data-dir` or `COMEMORY_DATA_DIR`). Full per-command docs
with flags and worked examples:
**[docs/cli-reference.md](docs/cli-reference.md)**.

---

## Configuration

Config is layered: built-in defaults → `~/.comemory/config.toml` → environment →
CLI flags. The full environment-variable table (data dir, retrieval top-k,
auto-reindex mode, ACT-R decay, MMR lambda, BM25 weights, prune floors, …) lives
in **[docs/configuration.md](docs/configuration.md)**; the ranking knobs and how
to tune them are walked through in
**[docs/guides/ranking-and-eval.md](docs/guides/ranking-and-eval.md)**.

---

## BYO-Vector workflow

comemory ships **without** a bundled embedding model — lexical search works
immediately, and you supply vectors via `--vector` (CSV) or `--vector-stdin`
(JSON `{"embedding":[..]}`) to add the dense leg (dims **1024** for `memory_vec`,
**768** for `code_vec`; mismatches fail fast with `VecDimMismatch`).

Full recipe, including the sample Ollama wrapper
[`scripts/comemory-embed.sh`](scripts/comemory-embed.sh):
**[docs/guides/byo-vectors.md](docs/guides/byo-vectors.md)**.

---

## Documentation

Start at the docs index — **[docs/README.md](docs/README.md)** — or jump to a
tier directly:

- **Tutorial** — [docs/getting-started.md](docs/getting-started.md): install,
  save, search, and index code in a few minutes.
- **How-to guides** —
  [byo-vectors](docs/guides/byo-vectors.md) ·
  [auto-reindex](docs/guides/auto-reindex.md) ·
  [ranking-and-eval](docs/guides/ranking-and-eval.md) ·
  [serve-web](docs/guides/serve-web.md) ·
  [prune-and-gc](docs/guides/prune-and-gc.md).
- **Reference** — [docs/cli-reference.md](docs/cli-reference.md): every
  subcommand and flag · [docs/configuration.md](docs/configuration.md): every
  environment variable and config knob.
- **Explanation** — [docs/architecture.md](docs/architecture.md): storage
  layout, retrieval pipeline, edge graph, save flow.
- **[CHANGELOG](CHANGELOG.md)** — what changed, version by version.

---

## Contributing

Read **[CLAUDE.md](CLAUDE.md)** first — it documents the architecture, the
module map, the frontmatter schema, and the **five binding rules** every
contribution must satisfy:

1. No duplication — shared logic is extracted.
2. Very modular modules — narrow, single-purpose files.
3. ≤ 300 code lines per file in `src/` (blanks/comments excluded).
4. Zero errors, zero warnings — no `#[allow]`, no bare `.unwrap()`, no
   `println!` in `src/`.
5. Tests strictly in `tests/`, mirroring `src/` 1:1 (flat, dunder-joined).

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
