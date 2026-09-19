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
# targets; anything else needs the source install below). `install.sh` is
# attached to every release; `latest/download` resolves to the newest one.
# `--proto '=https'` binds the redirect too — curl(1) on --proto-redir:
# "Protocols denied by --proto are not overridden by this option" — so the hop
# to the asset host cannot be downgraded to plaintext before the body reaches
# a shell.
curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL \
  https://github.com/Falconiere/comemory/releases/latest/download/install.sh | sh

# Piping into a shell runs whatever the URL serves. To read it first — or for
# a scripted install — see "Scripting the install" below, or take the
# checksum-verified archive route under "Verifying releases".

# Homebrew (macOS + Linuxbrew)
brew install Falconiere/tap/comemory

# From a local checkout (not published to crates.io)
git clone https://github.com/Falconiere/comemory && cd comemory
cargo install --path .
```

The installer detects the platform, resolves the newest release (or the one
you pin with `--version`), downloads the archive with a progress bar,
verifies it against the SHA-256 sidecar every release ships (`sha256sum`,
`shasum -a 256`, or `openssl` — it refuses to install unverified), checks the
binary actually runs on this machine (an old glibc fails here, with a
message, not later), swaps it into place atomically, and adds the bin
directory to your shell rc once, if it is not already on `PATH`. It re-runs
cleanly: a `comemory` already on `PATH` is replaced where it is, which is how
`comemory upgrade` moves you to the next release afterwards.

| Flag / env | Effect |
|---|---|
| `--version <tag>` / `COMEMORY_VERSION` | Install this release (`0.19.0` or `v0.19.0`) instead of the latest |
| `--dir <path>` / `COMEMORY_INSTALL_DIR` | Directory the binary goes in. Default: the existing `comemory`'s directory, else `$CARGO_HOME/bin` if it exists, else `~/.local/bin` |
| `--no-modify-path` / `COMEMORY_NO_MODIFY_PATH` | Leave shell rc files alone (a Dockerfile `RUN`, or when `--dir` is already on `PATH`) |
| `--quiet` | Only print errors |
| `NO_COLOR` | Plain output |

Scripting the install — a Dockerfile `RUN`, or any CI whose shell lacks
`pipefail` — download and run in two steps instead: a pipeline reports only the
shell's exit status, so a failed fetch leaves `curl … | sh` exiting 0 with
nothing installed. See [docs/getting-started.md](docs/getting-started.md#1-install).

The script itself is unsigned and unchecksummed; what it verifies is the
archive it downloads. The short URL `https://get.comemory.io/pkg/comemory/install`
(a redirect served from a separate repo) still points at cargo-dist's older
`comemory-installer.sh` until it is repointed at the asset above.

Then run `comemory setup`. It detects what this machine and the repo you are
in still need — the agent-host plugin, the git reindex hooks, a code index, a
cloud sign-in — shows you the plan, and applies the parts you pick. It never
redoes work that is already done, so it is safe to re-run; `--yes` applies
everything without prompting and `--dry-run --json` just reports.

Then verify: `comemory doctor`. Prebuilt binaries for **macOS aarch64** and
**Linux** (x86_64 + aarch64, gnu) are attached to every
[GitHub Release](https://github.com/Falconiere/comemory/releases), beside
`install.sh` and the cargo-dist generated `comemory-installer.sh` (the older
installer; still published, no in-place upgrade, no checksum on stock macOS).

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
layer; a SimHash near-dup check and `--supersedes` keep the store tidy. Saves
are content-addressed and idempotent: re-saving the same body returns the same
id and creates no second memory (`created: false` in `--json`).
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
| `comemory benchmark` | Score a reviewed benchmark set over memory, code and document retrieval; reports candidate-pool recall apart from recall@k / MRR / nDCG@k and writes a replayable artifact |
| `comemory judge` | Record reviewed relevance verdicts against a candidate observation `comemory find` captured, or report that observation |
| `comemory export-dataset` | Export the reviewed relevance dataset and its manifest as versioned JSONL, with grouped splits, a withheld holdout and a report of everything it refused to guess at |
| `comemory mine` | Distill failed→successful query rewordings into expansions (`--apply`) |
| `comemory tune` | Grid-search ranking knobs against the golden set (`--apply` writes `config.toml`) |
| `comemory bandit` | Thompson-sample ranking knobs (`--apply` writes when the sample beats baseline) |
| `comemory index-code` | Walk a repo, extract symbols, mine the co-change/import graph, run PageRank |
| `comemory ingest-code` | Read pre-embedded JSONL from stdin into the code index |
| `comemory graph` | Export the code-connection graph as JSON, Graphviz DOT, or interactive HTML |
| `comemory serve` | Loopback `/api/v1` HTTP server: every command over REST, background jobs with SSE progress |
| `comemory ast` | Run an ast-grep structural pattern against a source file |
| `comemory setup` | Detect what this machine and repo still need, then set it up |
| `comemory doctor` | Report on data-directory and SQLite-mirror health |
| `comemory prune` | Detect (and optionally soft-delete) stale memories |
| `comemory rebuild` | Drop `comemory.db` and repopulate it from `memories/*.md` |
| `comemory gc` | Purge old entries from `memories/.trash/` and aged telemetry |
| `comemory completions` | Generate shell completions |
| `comemory install-hooks` | Install git hooks that reindex code on commit/merge/checkout |
| `comemory hooks` | Report and toggle those hooks individually, plus search→edit reinforcement |
| `comemory upgrade` | Move this binary to the newest release (`--check` only reports; `--version` pins) |
| `comemory auth login` | Log in; mint an organization `cmk_` into `auth.json` and run the first sync (`--api-url`) |
| `comemory auth status` | Check whether local / `COMEMORY_API_KEY` credentials still authenticate |
| `comemory auth logout` | Delete local `auth.json` (no remote revoke) |
| `comemory sync` | Push/pull against the bound organization (`--action`, `--allow-secret`) |
| `comemory watch` | Follow the organization's changes over the workspace channel and pull on each (`--once`) |
| `comemory capture session` | Redact a coding-tool transcript and POST a receipt (`--path` / `--session-id` / `--from-hook`, `--dry-run`, `--allow-secret`) |
| `comemory capture sources` | List platform capture-consent rows (CLI cannot enable capture) |
| `comemory capture install-hook` | Install Claude Code `SessionEnd` → `capture session --from-hook` |
| `comemory distill` | Extract explicit `comemory save` claims from a transcript and propose platform candidates (`--session-id`, `--transcript`, `--dry-run`) |

Every command accepts `--json`; the data root defaults to `~/.comemory`
(overridable with `--data-dir` or `COMEMORY_DATA_DIR`). Full per-command docs
with flags and worked examples:
**[docs/cli-reference.md](docs/cli-reference.md)**.

---

Memory listing uses ordered indexes and trigram candidates for literal substring
queries of at least three characters, with exact scan behavior for shorter queries.
Existing databases gain these indexes automatically; see [storage architecture](docs/architecture.md).

## Offline retrieval benchmark

`comemory eval` scores memory-only lexical retrieval against a memory-id golden
file, and it is unchanged. `comemory benchmark` answers a different question —
how does ranking do across **memory, code, documents and mixed queries**, and
would reordering the candidates help?

```bash
comemory benchmark --set benchmark.yaml --report run.json
```

A benchmark set is a reviewed, versioned YAML file. It pins its own retrieval
configuration and its own budgets *before* anything is scored, so a result
cannot be graded against a threshold invented afterwards:

```yaml
version: 1
name: my-mixed-set
ranking:                 # pinned: decay 0.0 freezes ACT-R activation, so a run
  rrf_k: 60.0            # reproduces across days. Disabling access tracking
  decay: 0.0             # alone does NOT freeze decay.
  mmr_lambda: 0.7
  bm25_weights: [1.0, 3.0]
  graph_hops: 2
  graph_seeds: 8
defaults: { k: 5, max_text_bytes: 4096 }
budgets:
  min_tasks: 8           # fewer judged tasks => every verdict is inconclusive
  min_ndcg_gain: 0.02
  max_ndcg_regression: 0.01
  max_p95_task_ms: 1500
tasks:
  - id: mixed-01
    domain: all          # memory | code | document | all
    query: activation decay
    judgments:
      - relevance: 3     # graded 0..=3; 0 is "reviewed, not relevant"
        target: { domain: memory, id: 5a9f19bc }
```

What it reports, and why each piece exists:

- **Candidate-pool recall, separately from recall@k.** A miss retrieval never
  produced and a miss it ranked below the cut are different failures, and only
  the second is one a reranker can fix. A judged-relevant result retrieval did
  not return is reported as a miss, never inserted into the candidates.
- **Judgment coverage, stated outright.** `judged_in_page`,
  `unjudged_in_page` and `judged_page_fraction`, so a number computed over a
  thinly judged page is visible rather than implied.
- **Per-domain results.** Each corpus's own judgments are scored separately,
  and each filter narrows only the legs it belongs to — `kind` the memory leg,
  `lang` the code leg, `--path` globs the document leg. A task that sets a
  filter for a leg it does not run fails to load.
- **Arms over one candidate snapshot.** The deterministic baseline plus any
  number of scorer outputs supplied as JSON (`--scores`, repeatable), compared
  with paired bootstrap intervals. A verdict is read off the interval, and too
  few judged tasks is `inconclusive` whatever the point estimate says.
- **A replayable artifact.** `--report` writes every candidate observation —
  domain-qualified identity, content version, bounded text and its digest —
  so an external scorer can be run offline over exactly what retrieval
  produced, and its scores fed back as an arm.

A run writes no query log row and bumps no access counter: measurement never
feeds the signals it measures. The full contract, including the reference-string
encoding and the per-domain identity rules, is in
[docs/designs/2026-09-18-domain-aware-retrieval-benchmark.md](docs/designs/2026-09-18-domain-aware-retrieval-benchmark.md).

### Capturing real queries, and judging them

A benchmark set is hand-written. The same contract can also be filled in from
real queries, which is what `[observations]` turns on:

```bash
COMEMORY_OBSERVATIONS_ENABLED=1 comemory find "activation decay"
# … ranked hits …
# observation: o-20260918-9f8e7d6c  (judge: comemory judge --observation o-20260918-9f8e7d6c)

comemory judge o-20260918-9f8e7d6c            # what was in the pool
comemory judge o-20260918-9f8e7d6c \
  --ref 'memory:5a9f19bc:5a9f19bc403e…=3' \
  --ref 'document:0f1e2d3c:guides/chunking.md:7b2a9c…:3=1'
```

Capture is **off by default**, bounded by `observations.max_text_bytes` and
`observations.max_candidates`, and best effort: a search that cannot record an
observation still returns its hits. It never runs on a read-only
`comemory serve`, because it only arms on a run that may write telemetry.

A verdict addresses a candidate by its domain-qualified reference, never by a
title or a path on screen, so:

- a target the pool never returned is refused as a candidate-pool recall miss,
  and nothing in that call is written;
- a reference pinned to a content version the observation did not see is
  refused as **stale** — after an edit and re-index, a code reference still
  names the same `(repo, path, symbol)` and never the recyclable
  `code_symbols` rowid;
- documents get a relevance path they have never had.

`comemory gc` ages unjudged observations out on the learning-retention window
and keeps judged ones; purging a deleted memory blanks its captured passage
while keeping the recorded pool's shape. Observations are local — never synced.
The contract is
[docs/designs/2026-09-18-candidate-observation-capture.md](docs/designs/2026-09-18-candidate-observation-capture.md).

### Exporting what was reviewed

`comemory export-dataset` turns those observations and verdicts into a
versioned JSONL dataset plus a manifest, for the evaluator and for an external
trainer. No model framework enters the Rust binary.

```bash
comemory export-dataset --out ./dataset
# d-3c1f5a90b7e24d68 -> ./dataset  (312 record(s), 14 group(s), record v1)
#   train.jsonl                     210 row(s)     412233 byte(s)  5e11c0a7b3d2
#   validation.jsonl                 45 row(s)      88214 byte(s)  9a41f0be7712
#   holdout.jsonl                    45 row(s)      88120 byte(s)  c74003fa1b59  WITHHELD
# unjudged 900  unresolved 7 (judged 2)  stale 1  pool miss 0  …
```

The defaults are the conservative ones, and each is a rule rather than a
preference:

- **Reviewed verdicts only.** Implicit signals are exported only on request and
  only into their own `<split>.implicit.jsonl`, so a pseudo-label is never
  silently promoted to evaluation truth. Access frequency and the absence of a
  click are not negative relevance judgments.
- **A reviewed relevance of `0` is a hard negative; a candidate nobody judged
  stays unjudged.** The unjudged pool is reported in the manifest and emitted
  only under `--include-unjudged`, always with `label: null`.
- **An unresolved candidate is reported, never exported.** A purge blanks a
  captured passage in place; the row keeps the pool's shape, and the manifest
  counts both it and any verdict recorded against it.
- **Splits are assigned to connected components of the query/content graph,
  before any negative selection.** Two chunks of one document, two symbols of
  one file, two content versions of one memory and two spellings of one query
  never land on opposite sides. `--holdout-repo` and `--holdout-since` reserve
  a whole repository or time slice when the data allows it.
- **The holdout split is withheld.** Without `--include-holdout` no
  `holdout.jsonl` is written at all, and a stale one an earlier run left in the
  output directory is removed first; the manifest publishes only its row count
  and SHA-256.
- **A repeated export from an unchanged database is byte-identical**, and
  `dataset_id` plus `snapshot_digest` prove it.

The record and manifest schemas are in
[docs/designs/2026-09-18-reviewed-dataset-export.md](docs/designs/2026-09-18-reviewed-dataset-export.md).

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

```bash
comemory upgrade --check   # running vs newest release, nothing installed
comemory upgrade           # swap this binary for the newest release, in place
```

`upgrade` resolves the newest GitHub release, works out how this binary was
installed, and hands the swap to that release's own `install.sh` (Homebrew
installs go through `brew upgrade comemory`; a `cargo install` build is
rebuilt, and the command prints the recipe instead). `--version` pins a
release, `--force` allows a reinstall or downgrade, `--json` reports the
outcome as an object.

## Cloud auth (workspace key)

```bash
comemory auth login                  # device code → console /device → cmk_ in auth.json
comemory auth login --api-url URL    # or set COMEMORY_API (default https://api.comemory.io)
comemory auth status                 # GET /v1/workspaces/current with the saved key
comemory auth logout                 # delete local auth.json (no remote revoke)
```

Mints a **workspace-bound** `cmk_` (not a sync device key). Credentials live at
`$COMEMORY_DATA_DIR/auth.json` (mode `0600`). `COMEMORY_API_KEY` overrides the
file secret for scripting/CI.

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
  layout, retrieval pipeline, edge graph, save flow ·
  [designs/2026-09-18-domain-aware-retrieval-benchmark](docs/designs/2026-09-18-domain-aware-retrieval-benchmark.md):
  the offline benchmark and the candidate observation contract.
- **[CHANGELOG](CHANGELOG.md)** — what changed, version by version.

---

## Contributing

Runtime queries use the declared toolu-orm tables. See the
[runtime query guide](docs/guides/runtime-orm.md) for supported builders,
behavior-preserving conversions, and upstream issues for retained SQL.

Read **[AGENTS.md](AGENTS.md)** first — it documents the architecture, the
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

## Agent integration

`comemory install claude` and `comemory install codex` install the bundled skills
and hooks through the host plugin manager, and write the plugin's `.mcp.json` so
the host spawns `comemory mcp` — an eleven-tool MCP stdio server over the same
command cores as the CLI and `comemory serve`. Use `--dry-run` to preview. Any
other MCP host (Cursor, Gemini CLI, Windsurf) registers `comemory mcp` by hand
with a one-line stdio server entry. `comemory recall-status` reports what the
learning loop still owes a verdict. The integration is owned here and requires
no toolu plugin. See [installation and migration](docs/guides/agent-integration.md).
