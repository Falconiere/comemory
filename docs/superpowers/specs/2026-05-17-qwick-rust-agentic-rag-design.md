# qwick — Rust Rewrite, Agentic RAG Pivot (Design Spec)

**Status:** Draft for review
**Date:** 2026-05-17
**Author:** Falconiere R. Barbosa
**Supersedes:** all prior `qwick-memory` design docs (`2026-03-20-qwick-rag-design.md` and successors)

---

## 1. North Star

`qwick` is a Rust CLI that fuses three ideas:

- **Persistent dev-memory** in the spirit of [engram](https://github.com/Gentleman-Programming/engram) — decisions, bugs, conventions, discoveries survive sessions and projects.
- **Semantic code search** in the spirit of [grepai](https://github.com/yoanbernabeu/grepai) — natural-language queries over a repo's code, locally embedded, agent-friendly.
- **Structural AST patterns** in the spirit of [ast-grep](https://github.com/ast-grep/ast-grep) — tree-sitter-backed `$VAR` patterns for precise symbol extraction and code search.

These three are knit together by a **two-layer property graph** that connects memory artifacts to the live code they reference, exposed to the caller (typically Claude Code) as a **toolbox of sharp retrieval primitives**. Agency — query decomposition, reflection, synthesis — lives in the caller. The CLI runs deterministic retrieval, graph walks, and corrective fallback **without invoking any LLM**.

This is "Agentic RAG" in the **toolbox + graph** flavor: the canonical retrieve/reflect/refine/stop loop is implemented in Rust, deterministically, while the LLM-driven decomposition and synthesis stay in the caller. No external API spend per query, no in-process model inference.

This is a **full rewrite**, not a port. There is no migration from the existing Python `qwick-memory`. Existing memories, indices, and configuration are discarded.

---

## 2. Why This Pivot

Today's `qwick-memory` (Python + LanceDB + FastEmbed + FastMCP) is a competent engram-style memory store. Three forces motivate the rewrite:

1. **The caller is already an agent.** Claude Code orchestrates reasoning. Adding LLM-driven agency *inside* the CLI duplicates that orchestration and compounds cost. The right move is to give Claude Code **sharper tools** rather than another agent loop.
2. **Memories without code links are isolated.** A decision about `run_migration` should be reachable from the symbol `run_migration` and vice versa. Today's flat vector search can find both but can't traverse between them.
3. **Python + MCP plugin distribution has been painful.** Plugin venvs disappear, lancedb fails on concurrent launches, dependencies (lancedb, fastembed, future cross-encoder) bloat the install for every team member. A statically-linked Rust binary with one `cargo install qwick` is the right shape.

The rewrite also drops MCP. Claude Code calls `qwick` via Bash with `--json`. No MCP server, no plugin install, no shared venv.

---

## 3. Scope (v1.0.0)

In scope:

- Memory layer: save, search, list, delete, atomic markdown I/O, frontmatter schema v1.
- Memory graph: structural edges, semantic `relates_to`, manual `supersedes`/`conflicts_with`.
- Code layer: on-demand per-repo indexing via ast-grep, symbol-level embeddings, call/import graph.
- Cross-links: memory body → symbol resolution on save and on code reindex.
- Adaptive retrieval pipeline: deterministic router, hybrid vector+FTS, corrective fallback, graph walks.
- Headline command: `qwick context <symbol-or-id>` — one-shot bundle of code + memories + graph neighbors.
- Stale-data pruning: orphans, stale code chunks, low-value memories. Soft-delete via trash.
- Auto-update: git-assisted incremental reindexing in `lazy` or `hook` mode.
- Output: TTY default, `--json` flag. No MCP server.
- Stats: usage, feedback, irrelevance tracking via SQLite.
- Distribution: `cargo install`, prebuilt binaries via `cargo-dist`, Homebrew tap.

Deferred to v1.1+:

- LLM-driven `supersedes`/`conflicts_with` detection.
- Foreground file-watcher daemon (`qwick watch`).
- Cross-repo code indexing.
- Cross-encoder reranking (already specced for Python v2.1; will be re-specced for Rust).
- MCP shim (only if Bash-`qwick` proves insufficient).
- Cloud sync / multi-machine replication.
- TUI dashboard.

---

## 4. Architecture

### 4.1 High-level diagram

```
                  ┌─────────────────────────────────────┐
                  │            qwick (Rust CLI)         │
                  │                                     │
   stdin/args ──▶ │  clap parser ─▶ command dispatcher  │ ──▶ stdout (TTY or --json)
                  │       │                             │
                  │       ▼                             │
                  │  ┌────────────────────────────┐     │
                  │  │  Retrieval pipeline        │     │
                  │  │   adaptive router          │     │
                  │  │     │                      │     │
                  │  │     ▼                      │     │
                  │  │   vector + FTS + graph     │     │
                  │  │     │                      │     │
                  │  │     ▼                      │     │
                  │  │   corrective fallback      │     │
                  │  │     │                      │     │
                  │  │     ▼                      │     │
                  │  │   cited result bundle      │     │
                  │  └────────────────────────────┘     │
                  └───┬──────────┬─────────┬─────────┬──┘
                      │          │         │         │
                      ▼          ▼         ▼         ▼
                ┌─────────┐ ┌────────┐ ┌──────┐ ┌─────────┐
                │ lancedb │ │  kuzu  │ │stats │ │ astgrep │
                │ vectors │ │ graph  │ │.db   │ │  core   │
                └────┬────┘ └────┬───┘ └──────┘ └────┬────┘
                     │           │                   │
                     │           │                   ▼
                     │           │            ┌──────────┐
                     │           │            │tree-sitter│
                     │           │            └──────────┘
                     ▼           ▼
              ┌───────────────────────┐
              │  ~/.qwick/memories/   │
              │    {id}-{slug}.md     │ ← source of truth
              └───────────────────────┘
```

### 4.2 Component map

| Component | Responsibility |
|---|---|
| `cli` | clap subcommand definitions, arg parsing, dispatch, exit codes |
| `memory` | Markdown I/O, frontmatter parsing, atomic save, ID generation |
| `index` | LanceDB tables (memory + code), dual fastembed wrapper |
| `graph` | kuzu schema, node/edge upserts, multi-hop Cypher queries |
| `retrieval` | Adaptive router, hybrid vector+FTS, corrective fallback, ranking |
| `ast` | ast-grep wrapper, per-language symbol extractor, user pattern API |
| `stats` | SQLite — retrieval counts, feedback, irrelevance, repo index markers |
| `config` | Layered config: built-in defaults → `config.toml` → env → CLI flags |
| `output` | TTY rendering (owo-colors) + JSON serializers (serde_json) |
| `prune` | Orphan, stale-code, low-value detection and (soft) deletion |
| `git_utils` | Repo/author detection, blob OID lookup, hook installation |

### 4.3 Stack

```toml
# Cargo.toml (relevant dependencies)
lancedb           # vector index
kuzu              # embedded property graph (Cypher)
ast-grep-core     # symbol extraction + user patterns
tree-sitter       # parsers (pulled by ast-grep)
fastembed         # local embeddings (nomic + jina-code)
rusqlite          # stats + indexing markers
clap              # CLI parsing
tokio             # async runtime (lancedb is async)
serde             # core serde
serde_json        # JSON output mode
serde_yaml        # frontmatter
toml              # config
thiserror         # error enum
tracing           # structured logging
tracing-subscriber
git2              # repo/author detect, hook install, blob OID lookup
sha2              # content hashes
walkdir
ignore            # gitignore-aware walks
owo-colors        # TTY colors
```

Crates **deliberately not used:**

- **swiftide, rig** — both are agent frameworks; our design has no in-process LLM, so the value-add doesn't apply.
- **candle** — local model runtime; not needed without in-process LLM.
- **reqwest, openai-api crates** — no external API calls in v1.

---

## 5. Data Model

### 5.1 Storage layout

```
~/.qwick/
├── memories/{id}-{slug}.md      ← source of truth (markdown + frontmatter)
├── memories/.trash/{id}.md      ← soft-deleted memories, retained 30 days
├── index/
│   ├── vectors.lance/           ← LanceDB tables: memory_chunks, code_chunks
│   └── graph.kuzu/              ← kuzu database
├── stats.db                     ← SQLite: usage, feedback, repo markers
└── config.toml                  ← per-user configuration
```

Markdown is the single source of truth. Both indices are fully rebuildable from `memories/*.md` plus a re-scan of the target repo(s).

### 5.2 Frontmatter (schema v1)

```yaml
---
id: a1b2c3d4                         # 8-hex SHA-256 prefix of body
kind: decision                       # decision | bug | convention | discovery | pattern | note
repo: qwick-backend                  # primary repo; may be empty
tags: [postgres, migration]
author: falconiere
created: 2026-05-17T14:30:00Z
quality: 4                           # 1–5, default 3
schema: 1
content_hash: a1b2c3d4e5f6...        # sha-256 of normalized body
references:                          # indexer-managed; user-edited values preserved
  symbols:
    - qwick-backend:src/db.rs:run_migration
  files:
    - qwick-backend:src/db.rs
relations:                           # indexer- and user-managed
  supersedes: [<id>]
  conflicts_with: [<id>]
  derived_from: [<id>]
---

free-form markdown body…
```

User edits the body and optional fields (`kind`, `repo`, `tags`, `quality`). `references` and `relations` are normally managed by the indexer; user-edited values are preserved on reindex (treated as ground truth and merged).

### 5.3 LanceDB tables

```
memory_chunks:
  id           STRING   (PK)
  body         STRING
  embedding    VECTOR(768)   -- nomic-embed-text-v1.5
  kind         STRING
  repo         STRING
  tags         LIST<STRING>
  created      TIMESTAMP
  quality      INT
  content_hash STRING

code_chunks:
  qualified    STRING   (PK, format: <repo>:<path>:<symbol>)
  snippet      STRING
  embedding    VECTOR(768)   -- jina-embeddings-v2-base-code
  language     STRING
  file         STRING
  symbol_kind  STRING        -- function | method | class | type | const
  ast_hash     STRING
```

Two embedding models, two tables. Each table also gets a LanceDB FTS index on its text column (`body`, `snippet`) built at index time. Hybrid search queries vector + FTS in parallel, normalizes per-table scores, returns merged results.

### 5.4 Kuzu schema

```cypher
-- Nodes
NODE TABLE Memory(
  id STRING, kind STRING, created TIMESTAMP, quality INT,
  PRIMARY KEY(id)
)
NODE TABLE Repo(
  name STRING,
  last_indexed_head STRING, last_indexed_at TIMESTAMP,
  PRIMARY KEY(name)
)
NODE TABLE Author(name STRING, PRIMARY KEY(name))
NODE TABLE Tag(name STRING, PRIMARY KEY(name))
NODE TABLE File(
  qualified STRING,        -- <repo>:<path>
  repo STRING, path STRING,
  content_hash STRING,     -- git blob OID for tracked files
  indexed_at TIMESTAMP,
  PRIMARY KEY(qualified)
)
NODE TABLE Symbol(
  qualified STRING,        -- <repo>:<path>:<symbol>
  name STRING, kind STRING, language STRING,
  ast_hash STRING,
  PRIMARY KEY(qualified)
)

-- Memory-layer edges
REL TABLE InRepo(FROM Memory TO Repo)
REL TABLE AuthoredBy(FROM Memory TO Author)
REL TABLE Tagged(FROM Memory TO Tag)
REL TABLE Supersedes(FROM Memory TO Memory, at TIMESTAMP)
REL TABLE ConflictsWith(FROM Memory TO Memory)
REL TABLE RelatesTo(FROM Memory TO Memory, score DOUBLE)
REL TABLE DerivedFrom(FROM Memory TO Memory)

-- Code-layer edges
REL TABLE DefinedIn(FROM Symbol TO File)
REL TABLE Calls(FROM Symbol TO Symbol)
REL TABLE Imports(FROM File TO File)

-- Cross-layer edges (the killer feature)
REL TABLE ReferencesFile(FROM Memory TO File)
REL TABLE ReferencesSymbol(FROM Memory TO Symbol)
```

Multi-hop walks like *"all decisions referencing symbols defined in files imported by `src/db.rs`, ordered by supersedes chain"* become one Cypher query.

### 5.5 SQLite (stats)

```sql
CREATE TABLE retrieval_log(
  query_id     TEXT PRIMARY KEY,
  query        TEXT,
  returned_ids TEXT,    -- JSON array
  at           TIMESTAMP
);

CREATE TABLE feedback(
  memory_id    TEXT PRIMARY KEY,
  used_count   INTEGER DEFAULT 0,
  irrelevant_count INTEGER DEFAULT 0,
  last_used    TIMESTAMP
);

CREATE TABLE repo_marker(
  repo         TEXT PRIMARY KEY,
  last_head    TEXT,
  last_indexed_at TIMESTAMP
);
```

---

## 6. CLI Surface

All commands accept `--json` for machine-parseable output. TTY default uses `owo-colors` for readability. Exit codes follow `sysexits.h` conventions.

```
qwick save [--kind=decision] [--repo=X] [--tags=a,b] [--quality=4] [-]
qwick search "<nlq>" [--in=memory|code|both] [--repo=X] [--kind=...] [--limit=N] [--json]
qwick list [--kind=...] [--repo=X] [--json]
qwick delete <id>
qwick context <symbol-or-id> [--depth=1] [--json]      ← headline command
qwick symbol <name> [--lang=rs] [--json]
qwick memory-for <path>:<symbol> [--json]
qwick ast "<pattern>" [--lang=rs] [--rewrite=<...>]
qwick walk --from <id> --edge supersedes --depth 5 [--json]
qwick conflicts <id> [--json]
qwick supersedes <new-id> <old-id>
qwick feedback <query-id> --used <id,id> [--irrelevant <id,id>]
qwick context-recent [--token-budget=2000] [--json]
qwick index [--incremental]
qwick index-code [--incremental] [--include-dirty]
qwick prune [--orphans] [--stale-code] [--low-value [--below-quality=N --unused-since=180d]] [--apply]
qwick install-hooks [--force]
qwick gc                                                  # purge trash older than retention window
qwick doctor
qwick config get|set <key> [<value>]
```

`qwick context <symbol-or-id>` is the **headline command**: a single call that returns the symbol's source snippet, callers, callees, relevant memories (via cross-layer edges + vector search), and a graph-walked neighborhood — pre-packed so Claude Code can answer most "what about X?" questions in one round-trip.

---

## 7. Retrieval Pipeline

The pipeline runs entirely in Rust. No LLM calls. Implements the canonical agentic RAG control loop deterministically.

```
search("postgres migration race", in=both)
  │
  ├─ adaptive router (rule-based classifier)
  │   ├─ looks like a symbol identifier?         → symbol lookup + 1-hop graph
  │   ├─ has filters (--repo, --kind)?           → constrained vector
  │   ├─ short, factual, all stopwords removed?  → FTS-first
  │   └─ otherwise                                → hybrid (vector + FTS, parallel)
  │
  ├─ retrieve (parallel via tokio)
  │   ├─ lancedb memory_chunks (nomic embed)
  │   ├─ lancedb code_chunks (jina-code embed)   -- when in ∈ {code, both}
  │   ├─ kuzu graph walk from top-k seeds (1 hop, both layers)
  │   └─ sqlite stats join (usage boost, irrelevance penalty)
  │
  ├─ reflect (deterministic, no LLM)
  │   ├─ per-table score z-normalization
  │   ├─ relevance threshold filter (configurable per table)
  │   └─ confidence = top1_score − top2_score (gap signal)
  │
  ├─ refine (corrective fallback)
  │   ├─ if confidence < 0.15 AND results < 3:
  │   │       expand via graph RelatesTo from top seed → merge & re-rank
  │   └─ if results == 0 AND a strict filter was applied:
  │           drop the strictest filter, re-run once, mark "filter relaxed"
  │
  └─ stop and emit a cited bundle (id, score, kind, snippet, why)
```

Each step is a function on a pure `RetrievalState` struct. The pipeline is testable end-to-end without any external services.

---

## 8. Save Flow

```
qwick save "..." --kind=decision
  1. Parse args; build Memory struct; assign id = sha256(body)[:8].
  2. Write memories/.{id}.tmp (atomic stage).
  3. Embed body with nomic → upsert lancedb.memory_chunks.
  4. ast-grep against the current repo's code index:
       - resolve symbol references → frontmatter.references.symbols
       - resolve file references   → frontmatter.references.files
  5. kuzu upserts:
       - Memory node
       - InRepo, AuthoredBy, Tagged edges
       - ReferencesSymbol, ReferencesFile edges (cross-layer)
  6. lancedb cosine query top-5 neighbors above threshold → kuzu RelatesTo edges.
  7. Atomic rename memories/.{id}.tmp → memories/{id}-{slug}.md.
  8. git add + commit + push (best-effort, never fails the save).
```

On failure between steps 2–7: temp file deleted; partial kuzu/lancedb writes rolled back by deleting any rows keyed by `id`. The save is logically atomic from the caller's perspective.

---

## 9. Code Indexing Flow

```
qwick index-code [--incremental] [--include-dirty]
  1. cur_head      = git rev-parse HEAD
  2. last_head     = sqlite.repo_marker WHERE repo = $repo
  3. If cur_head == last_head and not --include-dirty: return early.
  4. changed       = git diff-tree --name-only $last_head $cur_head
                     (∪ git status --porcelain for working-tree, if --include-dirty)
  5. For each path in changed:
       a. If deleted: remove File, Symbol(s), code_chunks rows for that path.
       b. Else:
          - Get content hash via `git rev-parse :path` (free) or git hash-object.
          - If kuzu.File.content_hash matches: skip (no change).
          - Else: ast-grep parse, diff symbol set vs. existing Symbols.
                  - Upsert new/changed Symbols + DefinedIn + Calls + Imports edges.
                  - Embed new/changed symbol snippets via jina-code; upsert lancedb.
                  - Remove deleted Symbols, edges, and code_chunks rows.
  6. Update sqlite.repo_marker.last_head = cur_head.
  7. Re-resolve cross-layer references for memories pointing at this repo
     (cheap: graph lookup, no re-embed of memory bodies).
```

Working-tree (uncommitted) files are skipped by default; `--include-dirty` opts in. Output of `qwick context` marks symbols whose backing file is dirty.

---

## 10. Auto-Update Modes

Three configurable modes for keeping indices fresh:

```toml
[indexing]
auto_reindex = "lazy"               # "lazy" | "hook" | "off"
auto_reindex_threshold_ms = 200     # lazy bails if revalidation > this; warns instead
incremental_batch_size = 50
```

| Mode | Trigger | Behavior |
|---|---|---|
| `lazy` (default) | Before every `search` / `context` / `symbol` | Compare `git rev-parse HEAD` to `last_indexed_head`. If different and estimated cost < threshold, reindex incrementally in-line. Otherwise warn and proceed with stale index. |
| `hook` | git post-commit, post-merge, post-checkout | `qwick install-hooks` registers scripts that run `qwick index-code --incremental --quiet &` in the background after the event. |
| `off` | Manual only | `qwick index-code` when the user runs it. |

`qwick doctor` always reports the staleness gap (commits behind HEAD) for every known repo, regardless of mode.

---

## 11. Stale Data Pruning

Three kinds of stale data, three detection paths, one command surface.

| Stale | Cause | Detection |
|---|---|---|
| Orphan index entry | `.md` deleted but lancedb/kuzu row remains | scan: id in index ∧ id ∉ memories/ |
| Stale code chunk | source file deleted or content hash changed | re-`index-code`: file missing OR hash mismatch |
| Low-value memory | quality + usage + irrelevance threshold | sqlite join over feedback table |

```
qwick prune --orphans                                                   # dry-run, prints candidates
qwick prune --orphans --apply
qwick prune --stale-code --apply
qwick prune --low-value --below-quality=2 --unused-since=180d --apply
qwick prune --all --dry-run                                             # all three at once
```

Soft delete moves `memories/{id}.md` → `memories/.trash/{id}.md`. Retained 30 days, then purged by `qwick gc`. Index rows are hard-deleted (always rebuildable from markdown).

`qwick index --incremental` auto-prunes index orphans (does **not** delete markdown).
`qwick index-code --incremental` auto-prunes code chunks for deleted files.
`qwick doctor` reports stale counts read-only, never deletes.

---

## 12. Folder Structure

Single-crate layout. Workspace split deferred unless a clear sub-crate boundary emerges.

```
qwick/
├── Cargo.toml
├── Cargo.lock
├── deny.toml
├── rustfmt.toml
├── clippy.toml
├── typos.toml
├── lefthook.yml
├── justfile
├── .github/workflows/ci.yml
│
├── src/                              ← never contains test code
│   ├── main.rs
│   ├── lib.rs
│   ├── prelude.rs
│   ├── errors.rs
│   │
│   ├── cli/
│   │   ├── mod.rs
│   │   ├── save.rs
│   │   ├── search.rs
│   │   ├── context.rs
│   │   ├── symbol.rs
│   │   ├── memory_for.rs
│   │   ├── ast.rs
│   │   ├── walk.rs
│   │   ├── index.rs
│   │   ├── index_code.rs
│   │   ├── prune.rs
│   │   ├── feedback.rs
│   │   ├── conflicts.rs
│   │   ├── supersedes.rs
│   │   ├── list.rs
│   │   ├── delete.rs
│   │   ├── doctor.rs
│   │   ├── install_hooks.rs
│   │   └── config.rs
│   │
│   ├── memory/{mod,frontmatter,store,id}.rs
│   ├── index/{mod,memory_index,code_index,embedder,schema}.rs
│   ├── graph/{mod,schema,upsert,walk,query}.rs
│   ├── retrieval/{mod,router,hybrid,corrective,rank,bundle}.rs
│   ├── ast/{mod,extractor,pattern,languages}.rs
│   ├── stats/{mod,sqlite,feedback}.rs
│   ├── config/{mod,paths}.rs
│   ├── output/{mod,tty,json}.rs
│   ├── prune/{mod,orphans,stale_code,low_value}.rs
│   └── git_utils.rs
│
├── tests/                            ← all tests live here; mirrors src/ 1:1
│   ├── common/
│   │   ├── mod.rs                   ← fixtures, mock corpus, assert_cmd helpers
│   │   ├── corpus.rs
│   │   └── runner.rs
│   ├── memory.rs                    ← test binary entry; re-exports submodules
│   ├── memory/{frontmatter,store,id}.rs
│   ├── retrieval.rs
│   ├── retrieval/{router,hybrid,corrective,rank}.rs
│   ├── graph.rs
│   ├── graph/{upsert,walk}.rs
│   ├── index.rs
│   ├── index/{memory_index,code_index,embedder}.rs
│   ├── ast.rs
│   ├── ast/{extractor,pattern}.rs
│   ├── prune.rs
│   ├── prune/{orphans,stale_code,low_value}.rs
│   ├── stats.rs
│   ├── output.rs
│   ├── cli.rs                       ← assert_cmd + insta snapshots, all commands
│   ├── cross_link.rs                ← cross-cutting integration
│   └── e2e.rs                       ← real-binary happy path
│
├── benches/{search,index_code}.rs
├── docs/
│   ├── README.md
│   ├── architecture.md
│   └── superpowers/specs/
│       └── 2026-05-17-qwick-rust-agentic-rag-design.md
└── scripts/
    ├── e2e.sh
    └── seed-corpus.sh
```

**Test placement rule:** no `#[cfg(test)] mod tests { … }` block ever lives inside an `src/` file. Items needing tests are exposed with `pub(crate)` visibility. Each `tests/<module>.rs` is a thin test binary that pulls in `tests/<module>/<file>.rs` modules.

---

## 13. Test-Driven Development Discipline

Implementation follows the superpowers `test-driven-development` skill:

1. **Red → Green → Refactor per slice.** Every new function lands in three commits: failing test, passing implementation, optional refactor.
2. **Test pyramid:**
   - **Unit tests** (`tests/<module>/<file>.rs`): pure logic — frontmatter parse/emit round-trip, ID stability, score normalization, router classification, ast extraction per language.
   - **Integration tests** (`tests/<module>.rs`): module-level behavior against a tempdir-scoped `~/.qwick/`.
   - **Snapshot tests** (`insta`): JSON output of every CLI command; deterministic ordering enforced.
   - **Property tests** (`proptest`): ID stability under whitespace/unicode, frontmatter round-trip, rank monotonicity.
   - **End-to-end** (`scripts/e2e.sh`): real binary, full happy path, exercises save → index-code → context → search → prune → doctor.
3. **Coverage target:** 80% line coverage on `src/memory`, `src/retrieval`, `src/prune`, `src/graph`. Tracked via `cargo llvm-cov` in CI.
4. **No new feature merges without:** a failing test added in the red commit, passing in the green commit. Reviewer enforces.

---

## 14. Quality Gates

Toolchain:

| Tool | Purpose | Invocation |
|---|---|---|
| `rustfmt` | formatting | `cargo fmt --check` |
| `clippy` | lints | `cargo clippy --all-targets --all-features -- -D warnings` |
| `cargo-deny` | license + advisory + duplicate audit | `cargo deny check` |
| `cargo-machete` | unused dependency detection | `cargo machete` |
| `typos` | typo check on code + docs | `typos` |
| `cargo-nextest` | parallel test runner | `cargo nextest run --all-features` |
| `insta` | snapshot review | `cargo insta test --review` |
| `cargo-llvm-cov` | coverage | `cargo llvm-cov --lcov` |

Pre-commit via **lefthook**:

```yaml
# lefthook.yml
pre-commit:
  parallel: true
  commands:
    fmt:    { run: cargo fmt --check }
    clippy: { run: cargo clippy --all-targets -- -D warnings }
    typos:  { run: typos }
pre-push:
  commands:
    test:   { run: cargo nextest run --all-features }
    deny:   { run: cargo deny check }
```

CI (GitHub Actions): macOS-latest + ubuntu-latest, stable Rust. Steps: fmt → clippy → test (nextest) → deny → coverage → e2e.

Single human entry point — `justfile`:

```
just check     # fmt + clippy + typos + machete
just test      # nextest run --all-features
just qa        # check + test + deny + insta review
just bench     # criterion
```

---

## 15. Configuration

Layered precedence (last wins): built-in defaults → `~/.qwick/config.toml` → environment variables → CLI flags.

```toml
# ~/.qwick/config.toml (defaults shown)

[paths]
data_dir = "~/.qwick"

[git]
auto_sync = false             # auto commit+push memories
remote = ""                   # empty = local-only

[embeddings]
memory_model = "nomic-embed-text-v1.5-Q"
code_model   = "jina-embeddings-v2-base-code-Q"

[indexing]
auto_reindex = "lazy"         # "lazy" | "hook" | "off"
auto_reindex_threshold_ms = 200
incremental_batch_size = 50

[retrieval]
memory_threshold = 0.55       # cosine score floor
code_threshold = 0.50
hybrid_weight  = 0.65         # vector vs. FTS blend
top_k = 12
corrective_min_confidence = 0.15

[prune]
trash_retention_days = 30
low_value_default_unused_since_days = 180
low_value_default_below_quality = 2

[output]
json = false                  # default to TTY
color = "auto"                # "auto" | "always" | "never"
```

Env var overrides take the shape `QWICK_<SECTION>_<KEY>=value` (e.g. `QWICK_INDEXING_AUTO_REINDEX=hook`).

---

## 16. Distribution

- `cargo install qwick`
- Prebuilt binaries via `cargo-dist` → GitHub Releases for macOS (x86_64, aarch64), Linux (x86_64, aarch64), Windows (x86_64).
- Homebrew tap: `brew install sidegigllc/tap/qwick`.

All three on day 1.

---

## 17. Out-of-Scope (Explicit)

To prevent scope creep, these are explicitly **not** in v1:

- No in-process LLM (no Anthropic / OpenAI / local model calls). The CLI never costs an API dollar at query time.
- No MCP server, no `qwick mcp serve` subcommand. Reconsider only if `--json` + Bash proves insufficient.
- No file watcher (`qwick watch`). Lazy + git hooks cover the 95% case.
- No cross-repo code index. Each repo is indexed independently.
- No cross-encoder reranking. The Python `v2.1` plan is shelved and will be re-specced for Rust if and when needed.
- No data migration from the Python `qwick-memory`. Start fresh.
- No cloud sync. Local + optional git remote only.

---

## 18. Risks and Open Questions

- **Kuzu Rust API maturity.** Kuzu's Rust bindings are stable but the ecosystem is smaller than SQLite's. Mitigation: keep the kuzu surface area inside `src/graph/` so a migration to recursive-CTE SQLite is a one-module change.
- **fastembed-rs model availability.** `jina-embeddings-v2-base-code` is supported as of fastembed-rs current; if it changes, a single config swap to another code-tuned model is enough.
- **ast-grep-core API churn.** Pin a minor version; treat its symbol-extractor surface as a wrapped boundary in `src/ast/extractor.rs`.
- **Binary size.** Adding kuzu + jina-code + ast-grep + tree-sitter parsers could push the binary to ~80–120 MB. Use feature flags to make parsers optional (`--no-default-features --features rust,python,typescript`).
- **Initial indexing time on big repos.** Estimate 30s–2min per repo for a first `index-code` on a medium repo. Acceptable. Lazy mode hides this on subsequent runs.
- **TDD discipline at scale.** Test placement rule (never inline) costs some `pub(crate)` boilerplate. Accepted trade-off.

---

## 19. Success Criteria

v1.0.0 ships when:

- `qwick save`, `search`, `list`, `delete` work for the memory layer with full TDD coverage.
- `qwick index-code` builds a complete graph + code-chunk index for a real repo in under 5 minutes for ~100k LOC.
- `qwick context <symbol>` returns a coherent bundle (snippet + callers + callees + memories + 1-hop graph) in under 300 ms warm.
- `qwick search` (both layers, hybrid) returns top-12 in under 200 ms warm.
- Lazy auto-reindex revalidation costs under 50 ms on a clean working tree.
- All quality gates pass on CI for macOS + Linux.
- `qwick doctor` produces an actionable, plain-English report.
- A clean install via `cargo install qwick` works on a fresh machine and runs the happy path without manual configuration.

---

## 20. Implementation Plan

Implementation plan to be produced by the superpowers `writing-plans` skill after this spec is approved. Expected structure:

1. **Bootstrap** — Cargo project, justfile, lefthook, CI, dependency vendoring.
2. **Memory core** — `src/memory/*` + tests, schema v1 frontmatter, atomic save.
3. **Stats** — `src/stats/*` + tests, SQLite schema.
4. **Indexing — memory side** — `src/index/memory_index.rs` + embedder + tests.
5. **Graph — memory layer** — kuzu schema, Memory/Tag/Author/Repo upserts, RelatesTo via cosine.
6. **Retrieval — memory only** — `src/retrieval/*` for memory-only hybrid + corrective fallback.
7. **CLI — memory-only commands** — `save`, `search`, `list`, `delete`, `feedback`, `context-recent`, `doctor`.
8. **AST layer** — `src/ast/*`, multi-language extractor, user pattern API.
9. **Indexing — code side** — `src/index/code_index.rs`, symbol embeddings, incremental algorithm.
10. **Graph — code layer + cross-links** — File/Symbol/Calls/Imports, ReferencesFile/Symbol edges, save-time cross-link resolution.
11. **Retrieval — both layers** — extend pipeline for code, dual-table merge.
12. **CLI — code commands** — `index-code`, `symbol`, `memory-for`, `ast`, `context`.
13. **Graph walks** — `walk`, `conflicts`, `supersedes` commands.
14. **Pruning** — orphans, stale-code, low-value; trash + gc.
15. **Auto-reindex** — lazy mode, install-hooks, doctor staleness reports.
16. **Output polish** — TTY rendering, JSON serializers, exit codes.
17. **Distribution** — cargo-dist setup, Homebrew tap, release workflow.
18. **Docs** — README, architecture.md, CLI reference.

Each slice gates on its tests passing and quality gates green. No slice ships partially.
