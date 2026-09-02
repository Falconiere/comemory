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
| 🔗 **Versioned code references** | `save --ref-file` / `--ref-symbol` pin a memory to code at a git anchor (blob + commit); `context` flags each link `fresh` / `stale` / `ghost`. See [linking code to memories](docs/guides/linking-code-to-memories.md). |
| 🧠 **Memory that decays** | ACT-R activation (recency × access count) and Beta-smoothed feedback rerank results the way human memory actually surfaces things. |
| 📈 **A real learning loop** | Record which hits helped → score recall@k / MRR → mine reformulations → grid-search (`tune`) or Thompson-sample (`bandit`) the ranking knobs. Auto search→edit reinforcement on `index-code`. All offline, all deterministic. |
| 🌐 **Local HTTP API** | `comemory serve` exposes every command as a loopback-only, token-gated `/api/v1` REST surface — the same cores the CLI runs, plus jobs with progress/log streaming — for the console and any local agent or script. |
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
# One-liner (macOS aarch64 + Linux gnu, glibc >= 2.35 — the only prebuilt
# targets; anything else needs the source install below).
# Resolves to the latest release's installer.
# `--proto '=https'` binds the redirect too — curl(1) on --proto-redir:
# "Protocols denied by --proto are not overridden by this option" — so the hop
# to GitHub cannot be downgraded to plaintext before the body reaches a shell.
curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL \
  https://get.comemory.io/pkg/comemory/install | bash
# cargo-dist generates that script for /bin/sh (dash/ash), so `| sh` works
# just as well — use it on an image that ships no bash.

# Piping into a shell runs whatever the URL serves. To read it first — or for
# a scripted install — see "Scripting the install" below, or take the
# checksum-verified archive route under "Verifying releases".

# Homebrew (macOS + Linuxbrew)
brew install Falconiere/tap/comemory

# From a local checkout (not published to crates.io)
git clone https://github.com/Falconiere/comemory && cd comemory
cargo install --path .
```

Scripting the install — a Dockerfile `RUN`, or any CI whose shell lacks
`pipefail` — download and run in two steps instead: a pipeline reports only the
shell's exit status, so a failed fetch leaves `curl … | bash` exiting 0 with
nothing installed. See [docs/getting-started.md](docs/getting-started.md#1-install).

What the one-liner verifies: the installer checks the archive it downloads
against a SHA-256 baked into itself, but **skips that check on stock macOS**,
which ships `shasum` rather than the `sha256sum` it needs. The installer script
itself is unsigned and unchecksummed, and is reached through a redirect served
from a separate repo.

Then verify: `comemory doctor`. Prebuilt binaries for **macOS aarch64** and
**Linux** (x86_64 + aarch64, gnu) are attached to every
[GitHub Release](https://github.com/Falconiere/comemory/releases), along with
a shell installer — the same script the one-liner above redirects to, fetched
without the redirect, so everything said about what it verifies applies here
too:

```bash
curl --proto '=https' --proto-redir '=https' --tlsv1.2 -LsSf \
  https://github.com/Falconiere/comemory/releases/latest/download/comemory-installer.sh | sh
```

Windows users fork the repo and run `cargo install --path .` — see
[Platform support](#platform-support) below.

Full install details — every channel, and generating shell completions — are in
**[docs/getting-started.md](docs/getting-started.md)**; the binary-size history
lives in **[docs/build-perf.md](docs/build-perf.md)**.

### Platform support

| Platform | Install |
|---|---|
| **macOS aarch64** (Apple Silicon) | Prebuilt: `brew install Falconiere/tap/comemory`, the shell installer above, or download from the latest [GitHub Release](https://github.com/Falconiere/comemory/releases) |
| **Linux x86_64 / aarch64** (gnu, **glibc ≥ 2.35**) | Prebuilt: the shell installer above, `brew install Falconiere/tap/comemory` (Linuxbrew), or download from the latest [GitHub Release](https://github.com/Falconiere/comemory/releases) |
| **Linux (other arch/libc, e.g. musl)** | Fork the repo and `cargo install --path .` |
| **Windows** | Fork the repo and `cargo install --path .` |

The release CI matrix builds `aarch64-apple-darwin`,
`x86_64-unknown-linux-gnu`, and `aarch64-unknown-linux-gnu`. If you need a
prebuilt for another platform, run `cargo dist build --target <triple>` from
a fork — cargo-dist is already wired up; only the published `targets` list
is narrowed.

### Verifying releases

Every release publishes `sha256.sum` — one line per tarball (each platform
archive plus `source.tar.gz`) — with a per-archive `<archive>.sha256` beside it.

```bash
base=https://github.com/Falconiere/comemory/releases/latest/download
archive=comemory-x86_64-unknown-linux-gnu.tar.xz   # swap for your platform

curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fL -O "$base/$archive"
curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fL -O "$base/sha256.sum"

# Verify that archive's line — `sha256.sum` lists every platform. Match the
# whole name field (`<hash> *<name>`), not a substring, and the `-s` guard makes
# a name that is not in the file fail instead of verifying nothing.
awk -v a="$archive" '$2 == "*" a || $2 == a' sha256.sum > line.sum \
  && [ -s line.sum ] \
  && sha256sum -c line.sum   # macOS: shasum -a 256 -c line.sum
```

No release carries a minisign signature today, and `keys/comemory.pub` is not
committed, so signature verification is not available; the checksum above is the
whole of it.

---

## Quickstart

```bash
comemory save "Use Postgres for analytics, not ClickHouse — see ADR-14" \
  --kind decision --repo myrepo --tags db,postgres   # capture a memory
comemory index-code --repo myrepo --path .           # symbols + graph + PageRank
comemory search "what database do we use"            # recall memories (lexical)
comemory search-code "connection pool retry" --repo myrepo   # ranked code search
comemory context run_migration --json                # source + memories + neighbors
comemory serve                                       # the same, over HTTP (/api/v1)
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
A deterministic **learning loop** (`feedback → eval → mine → tune|bandit`) measures and
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
| `comemory find` | One ranked list across memories, code, and documents (`--domain` narrows it) |
| `comemory context` | One-shot bundle for a key: code symbol + related memories + neighborhood |
| `comemory list` | List memories with optional repo/kind filters (`--sort created\|quality\|accessed`) |
| `comemory show` | Show one memory in full: body, frontmatter, activation, reference freshness |
| `comemory stats` | Corpus counters (memories, symbols, edges, documents) and database size |
| `comemory repos` | Indexed code repositories with their index freshness and changed-file count |
| `comemory delete` | Soft-delete a memory by id (moves to `.trash/`) |
| `comemory feedback` | Record per-hit feedback against a `query_id` (`--used` / `--used-code` …) |
| `comemory eval` | Score retrieval quality (recall@k, MRR) against a golden set (`--history` reads past runs) |
| `comemory mine` | Distill failed→successful query rewordings into expansions (`--apply`) |
| `comemory tune` | Grid-search ranking knobs against the golden set (`--apply` writes `config.toml`) |
| `comemory bandit` | Thompson-sample ranking knobs (`--apply` writes when the sample beats baseline) |
| `comemory index-code` | Walk a repo, extract symbols, mine the co-change/import graph, run PageRank |
| `comemory ingest-code` | Read pre-embedded JSONL from stdin into the code index |
| `comemory graph` | Export the code-connection graph as JSON, Graphviz DOT, or interactive HTML |
| `comemory serve` | Loopback `/api/v1` HTTP server: every command over REST, background jobs with SSE progress |
| `comemory ast` | Run an ast-grep structural pattern against a source file |
| `comemory doctor` | Report on data-directory and SQLite-mirror health |
| `comemory prune` | Detect (and optionally soft-delete) stale memories |
| `comemory rebuild` | Drop `comemory.db` and repopulate it from `memories/*.md` |
| `comemory gc` | Purge old entries from `memories/.trash/` and aged telemetry |
| `comemory completions` | Generate shell completions |
| `comemory install-hooks` | Install git hooks that reindex code on commit/merge/checkout |
| `comemory hooks` | Report and toggle those hooks individually, plus search→edit reinforcement |

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

## Upgrading

Point a newer `comemory` binary at an existing `~/.comemory` and the schema
migrates automatically on your next command — there is no `comemory migrate`
step to remember. Before any pending migration, comemory snapshots
`comemory.db` to `comemory.db.pre-v<N>.bak` first (skip with
`COMEMORY_SKIP_MIGRATION_BACKUP=1`); a snapshot *failure* only refuses the
upgrade when a pending migration could destroy data, and merely warns
otherwise. An older binary opening a database written by a newer one refuses
cleanly instead of corrupting it. Full story, including how to restore a
snapshot and the `comemory serve` restart caveat:
**[docs/guides/upgrading.md](docs/guides/upgrading.md)**.

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
  [http-api](docs/guides/http-api.md) ·
  [prune-and-gc](docs/guides/prune-and-gc.md) ·
  [upgrading](docs/guides/upgrading.md).
- **Reference** — [docs/cli-reference.md](docs/cli-reference.md): every
  subcommand and flag · [docs/configuration.md](docs/configuration.md): every
  environment variable and config knob.
- **Explanation** — [docs/architecture.md](docs/architecture.md): storage
  layout, retrieval pipeline, edge graph, save flow.
- **[CHANGELOG](CHANGELOG.md)** — what changed, version by version.

---

## Contributing

Read **[CLAUDE.md](CLAUDE.md)** first — it documents the architecture, the
module map, the frontmatter schema, and the **binding rules** every
contribution must satisfy (comemory follows the
[toolu-conventions](https://github.com/Falconiere/toolu-conventions) Rust
stack, plus its own stricter local ceilings):

1. No duplication — shared logic is extracted.
2. No barrels — no `mod.rs`; a grown module is `src/<name>.rs` beside
   `src/<name>/`.
3. One responsibility per file, filename matches content.
4. ≤ 300 code lines per file in `src/` (blanks/comments excluded), ≤ 100
   lines per function.
5. Zero errors, zero warnings — no `#[allow]`, no bare `.unwrap()`, no
   `println!` in `src/`, every `unsafe` carries a `// SAFETY:` comment.
6. Tests never share a file with production logic — colocated by default in
   a sibling `src/<module>/tests/` folder, with the CLI surface, `assert_cmd`,
   and `insta`-snapshot suites staying at crate-root `tests/`.
7. Every CLI flag and every `/api/v1` route has a scenario in
   [`docs/scenarios/`](docs/scenarios/README.md) that names the test covering
   it; `tests/cli_scenario_catalog.rs` fails when one goes missing.

The umbrella quality gate is one command — CI runs the same scripts:

```bash
bash scripts/check-all.sh     # fmt · type · lint · guardrails · typos · cli-docs · migration-check
just check                    # alias of the above
just test                     # cargo nextest run --all-features
just qa                       # check-all + cargo-deny + dup-check + machete
just e2e                      # real-binary end-to-end harness
```

A task isn't done until `scripts/check-all.sh` exits 0.

---

## License

[MIT](LICENSE) © Falconiere Barbosa

<div align="center">
<sub>Built in Rust 🦀 · 100% local · one binary · markdown is the source of truth</sub>
</div>
