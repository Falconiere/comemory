# Architecture overview

This is a 2-page on-ramp into the qwick-memory design. For full detail, every
section here links back to the corresponding section in the
[design spec](superpowers/specs/2026-05-17-qwick-rust-agentic-rag-design.md).

## 1. High-level diagram (spec §4.1)

```
                  ┌─────────────────────────────────────┐
                  │            qwick-memory (Rust CLI)         │
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
              │  ~/.qwick-memory/memories/   │
              │    {id}-{slug}.md     │ ← source of truth
              └───────────────────────┘
```

## 2. Component map (spec §4.2)

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

## 3. Storage layout (spec §5.1)

```
~/.qwick-memory/
├── memories/{id}-{slug}.md      ← source of truth (markdown + frontmatter)
├── memories/.trash/{id}.md      ← soft-deleted memories, retained 30 days
├── index/
│   ├── vectors.lance/           ← LanceDB tables: memory_chunks, code_chunks
│   └── graph.kuzu/              ← kuzu database
├── stats.db                     ← SQLite: usage, feedback, repo markers
└── config.toml                  ← per-user configuration
```

Markdown is the single source of truth. Both indices are fully rebuildable
from `memories/*.md` plus a re-scan of the target repo(s).

## 4. Data model snapshot (spec §5)

Frontmatter (schema v1):

```yaml
---
id: a1b2c3d4                         # 8-hex SHA-256 prefix of body
kind: decision                       # decision | bug | convention | discovery | pattern | note
repo: qwick-backend
tags: [postgres, migration]
author: falconiere
created: 2026-05-17T14:30:00Z
quality: 4                           # 1–5, default 3
schema: 1
content_hash: a1b2c3d4e5f6...
references:                          # indexer-managed
  symbols: [qwick-backend:src/db.rs:run_migration]
  files:   [qwick-backend:src/db.rs]
relations:                           # indexer- and user-managed
  supersedes: [<id>]
  conflicts_with: [<id>]
  derived_from: [<id>]
---
```

LanceDB has two tables:

- `memory_chunks` — `id`, `body`, `embedding` (nomic-embed-text-v1.5, 768d),
  `kind`, `repo`, `tags`, `created`, `quality`, `content_hash`.
- `code_chunks` — `qualified` (`<repo>:<path>:<symbol>`), `snippet`,
  `embedding` (jina-embeddings-v2-base-code, 768d), `language`, `file`,
  `symbol_kind`, `ast_hash`.

Each text column also gets a LanceDB FTS index for hybrid vector+FTS scoring.

Kuzu holds the two-layer graph:

- **Memory layer:** `Memory`, `Repo`, `Author`, `Tag` nodes; `InRepo`,
  `AuthoredBy`, `Tagged`, `Supersedes`, `ConflictsWith`, `RelatesTo`,
  `DerivedFrom` edges.
- **Code layer:** `File`, `Symbol` nodes; `DefinedIn`, `Calls`, `Imports`
  edges.
- **Cross-layer (the killer feature):** `ReferencesFile`, `ReferencesSymbol`
  edges. Multi-hop queries like *"all decisions referencing symbols defined
  in files imported by src/db.rs"* become a single Cypher query.

SQLite (`stats.db`) tracks `retrieval_log`, `feedback` (used / irrelevant
counts), and `repo_marker` (last indexed HEAD per repo).

## 5. Retrieval pipeline (spec §7)

The pipeline runs entirely in Rust. No LLM calls. It implements the
canonical agentic RAG control loop deterministically:

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

Each step is a pure function on a `RetrievalState` struct, so the pipeline
is end-to-end testable without external services.

## 6. Save flow (spec §8)

```
qwick-memory save "..." --kind=decision
  1. Parse args; build Memory; assign id = sha256(body)[:8].
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

On failure between steps 2–7 the temp file is deleted and any partial kuzu
or LanceDB rows keyed by `id` are rolled back. The save is logically atomic
from the caller's perspective. (See README "Known v1.1 gaps" for the
current scope of steps 4–6.)

## 7. Code indexing flow (spec §9)

```
qwick-memory index-code [--incremental] [--include-dirty]
  1. cur_head  = git rev-parse HEAD
  2. last_head = sqlite.repo_marker WHERE repo = $repo
  3. If cur_head == last_head and not --include-dirty: return early.
  4. changed   = git diff-tree --name-only $last_head $cur_head
                 (∪ git status --porcelain for working-tree, if --include-dirty)
  5. For each path in changed:
       a. If deleted: remove File, Symbol(s), code_chunks rows for that path.
       b. Else:
          - Hash the blob (git rev-parse :path or git hash-object).
          - If kuzu.File.content_hash matches: skip.
          - Else: ast-grep parse, diff symbol set vs. existing Symbols.
                  Upsert new/changed Symbols + DefinedIn + Calls + Imports.
                  Embed new/changed symbol snippets via jina-code → lancedb.
                  Remove deleted Symbols, edges, and code_chunks rows.
  6. Update sqlite.repo_marker.last_head = cur_head.
  7. Re-resolve cross-layer references for memories pointing at this repo.
```

Working-tree (uncommitted) files are skipped by default; `--include-dirty`
opts in. `qwick-memory context` marks symbols whose backing file is dirty.

## 8. Auto-update modes (spec §10)

Three configurable modes for keeping indices fresh:

```toml
[indexing]
auto_reindex = "lazy"               # "lazy" | "hook" | "off"
auto_reindex_threshold_ms = 200
incremental_batch_size = 50
```

| Mode | Trigger | Behavior |
|---|---|---|
| `lazy` (default) | Before every `search` / `context` / `symbol` | Compare `git rev-parse HEAD` to `last_indexed_head`. If different and estimated cost < threshold, reindex incrementally in-line. Otherwise warn and proceed with stale index. |
| `hook` | git `post-commit`, `post-merge`, `post-checkout` | `qwick-memory install-hooks` registers scripts that run `qwick-memory index-code --incremental --quiet &` in the background after the event. |
| `off` | Manual only | `qwick-memory index-code` runs only when the user invokes it. |

`qwick-memory doctor` always reports the staleness gap (commits behind HEAD) for
every known repo, regardless of mode.

## 9. Pruning (spec §11)

Three kinds of stale data, three detection paths, one command surface:

| Stale | Cause | Detection |
|---|---|---|
| Orphan index entry | `.md` deleted but lancedb/kuzu row remains | scan: id in index ∧ id ∉ memories/ |
| Stale code chunk | source file deleted or content hash changed | re-`index-code`: file missing OR hash mismatch |
| Low-value memory | quality + usage + irrelevance threshold | sqlite join over feedback |

Soft delete moves `memories/{id}.md` → `memories/.trash/{id}.md`. Trash is
retained 30 days, then purged by `qwick-memory gc`. Index rows are hard-deleted
(always rebuildable from markdown).

`qwick-memory index-code --incremental` auto-prunes code chunks for deleted files.
`qwick-memory doctor` reports stale counts read-only, never deletes.

## Where to go next

- [CLI reference](cli-reference.md) — every command with worked examples.
- [Design spec](superpowers/specs/2026-05-17-qwick-rust-agentic-rag-design.md) —
  full sections including configuration (§15), distribution (§16), and the
  risk register (§18).
