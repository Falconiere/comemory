# Architecture overview

This is a 2-page on-ramp into the comemory v0.2 design. The authoritative
write-up lives in
[`docs/superpowers/specs/2026-06-07-lightweight-v2-design.md`](superpowers/specs/2026-06-07-lightweight-v2-design.md);
this page mirrors the highlights for quick reference.

## 1. High-level diagram

```
                  ┌─────────────────────────────────────┐
                  │            comemory (Rust CLI)         │
                  │                                     │
   stdin/args ──▶ │  clap parser ─▶ command dispatcher  │ ──▶ stdout (TTY or --json)
                  │       │                             │
                  │       ▼                             │
                  │  ┌────────────────────────────┐     │
                  │  │  Retrieval pipeline        │     │
                  │  │   adaptive router          │     │
                  │  │     │                      │     │
                  │  │     ▼                      │     │
                  │  │   vector + FTS + edges     │     │
                  │  │     │                      │     │
                  │  │     ▼                      │     │
                  │  │   corrective fallback      │     │
                  │  │     │                      │     │
                  │  │     ▼                      │     │
                  │  │   cited result bundle      │     │
                  │  └────────────────────────────┘     │
                  └───────────────┬─────────────────────┘
                                  │
                                  ▼
                       ┌──────────────────────┐
                       │  ~/.comemory/           │
                       │   ├── memories/      │ ← source of truth
                       │   │    {id}-{slug}.md │
                       │   └── comemory.db       │ ← SQLite (everything else)
                       └──────────────────────┘
                                  │
                                  ▼
                       ┌──────────────────────┐
                       │  comemory.db tables     │
                       │   memories            │
                       │   memory_fts (FTS5)   │
                       │   memory_vec (vec0)   │
                       │   code_symbols        │
                       │   code_fts  (FTS5)    │
                       │   code_vec  (vec0)    │
                       │   edges               │
                       │   schema_meta         │
                       │   retrieval_log /     │
                       │   feedback / repo_*   │
                       └──────────────────────┘
```

## 2. Component map

| Component | Responsibility |
|---|---|
| `cli` | clap subcommand definitions, arg parsing, dispatch, exit codes |
| `memory` | Markdown I/O, frontmatter parsing, atomic save, ID generation |
| `store` | SQLite connection layer, schema_meta, migrations, vector + FTS helpers |
| `simhash` | 64-bit SimHash + Hamming distance over tokenized memory bodies |
| `graph` | SQL-backed edges (`Supersedes`, `ConflictsWith`, `RelatesTo`, `ReferencesFile`, `ReferencesSymbol`) + recursive walks; `cross_link` parses backticked refs |
| `retrieval` | Adaptive router, RRF-fused vector+FTS hybrid, corrective fallback, ranking |
| `ast` | ast-grep wrapper (rust/ts/js/py/go), per-language symbol extractor, user pattern API |
| `stats` | rusqlite usage / feedback / repo-marker tables (lives inside the same DB) |
| `config` | Layered config: built-in defaults → `config.toml` → env → CLI flags |
| `output` | TTY rendering (owo-colors) + JSON serializers (serde_json) |
| `prune` | Orphan, stale-code, low-value detection and (soft) deletion |
| `git_utils` | Repo/author detection, blob OID lookup, hook installation |

## 3. Storage layout

```
~/.comemory/
├── memories/{id}-{slug}.md      ← source of truth (markdown + frontmatter)
├── memories/.trash/{id}.md      ← soft-deleted memories, retained 30 days
├── comemory.db                     ← single SQLite file (see §3.1)
└── config.toml                  ← per-user configuration
```

Markdown is the single source of truth. `comemory.db` is fully rebuildable
from `memories/*.md` (plus a re-walk of indexed repos) via
`comemory rebuild`.

### 3.1 Inside `comemory.db`

One SQLite file replaces v0.1's `lancedb/`, `kuzu/`, and `stats.db` trio.
The database is created on first use, extended with the `sqlite-vec`
extension at runtime, and version-tracked through `schema_meta` so future
migrations stay idempotent.

| Table | Purpose |
|---|---|
| `schema_meta` | Single-row schema version + locked-in vector dimensions |
| `memories` | Frontmatter + body mirror keyed by memory id |
| `memory_fts` (FTS5) | Lexical index over memory body + title |
| `memory_vec` (vec0) | Dense vectors keyed by memory id; dim locked at first save |
| `code_symbols` | Symbols extracted from indexed repos (file, kind, snippet, ast_hash) |
| `code_fts` (FTS5) | Lexical index over symbol identifiers + snippets |
| `code_vec` (vec0) | Dense vectors for code symbols; dim locked at first ingest |
| `edges` | Sparse table replacing the kuzu graph (typed src→dst rows + payload) |
| `retrieval_log`, `feedback`, `repo_marker` | Stats / indexing markers carried over from v0.1 |

Every dense lookup goes through `sqlite-vec`'s `vec0` virtual table with a
dimension guard so a mismatched embedder fails fast (`VecDimMismatch`)
instead of corrupting the index. FTS5 hits and vector hits are fused via
Reciprocal Rank Fusion (RRF, `k = 60` by default).

## 4. Data model snapshot

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

The two `*_vec` tables hold caller-supplied vectors. `comemory` never
embeds locally; pass vectors via `--vector` / `--vector-stdin` (see the
"BYO-Vector workflow" section in the README). `COMEMORY_VECTOR_DIM` and
`COMEMORY_CODE_VECTOR_DIM` set the locked dimensionality;
`COMEMORY_EMBED_HINT` records (and surfaces in `comemory doctor`) the
identifier of the embedder you used.

The `edges` table is a flat `(src_kind, src_id, edge_kind, dst_kind, dst_id)`
schema that replaces the v0.1 kuzu graph for the small set of edges we
actually use (`Supersedes`, `ConflictsWith`, `RelatesTo`, `ReferencesFile`,
`ReferencesSymbol`). Multi-hop traversals use SQLite recursive CTEs.

## 5. Retrieval pipeline

The pipeline runs entirely in Rust. No LLM calls.

```
search("postgres migration race")
  │
  ├─ adaptive router
  │   ├─ symbol-like identifier?            → code symbol lookup + edge walk
  │   ├─ has filters (--repo, --kind)?      → constrained branch
  │   ├─ short, factual?                    → FTS5-first
  │   └─ otherwise                          → hybrid (vector + FTS5, fused via RRF)
  │
  ├─ retrieve
  │   ├─ memory_vec / code_vec (if vector supplied)
  │   ├─ memory_fts / code_fts (always)
  │   ├─ edges walk from top-k seeds (1 hop, recursive CTE)
  │   └─ stats join (usage boost, irrelevance penalty)
  │
  ├─ reflect (deterministic)
  │   ├─ RRF fusion across vector + FTS branches
  │   ├─ per-table threshold filter
  │   └─ confidence = top1_score − top2_score
  │
  ├─ refine (corrective fallback)
  │   ├─ if low confidence: expand via RelatesTo edges → merge & re-rank
  │   └─ if zero results + strict filter: drop strictest filter, retry once
  │
  └─ emit a cited bundle (id, score, kind, snippet, why)
```

If no vector is supplied, the vector branch is skipped and the router
returns the FTS5 ranking unchanged.

## 6. Save flow

```
comemory save "..." --kind=decision [--vector ... | --vector-stdin]
  1. Parse args; build Memory; assign id = sha256(body)[:8].
  2. Write memories/.{id}.tmp (atomic stage).
  3. SQLite upsert (inside one transaction):
       - memories row
       - memory_fts row
       - memory_vec row (only if a vector was supplied; dim guard runs first)
       - edges from cross_link::extract_refs (ReferencesFile / ReferencesSymbol)
  4. Atomic rename memories/.{id}.tmp → memories/{id}-{slug}.md.
  5. git add + commit + push (best-effort, only when COMEMORY_GIT_AUTO_SYNC is on).
```

Markdown is always the source of truth. If the SQLite transaction fails,
the markdown file is removed and the caller sees the original error;
`comemory rebuild` can always reconstruct the DB from `memories/*.md`.

## 7. Code indexing flow

```
comemory index-code --repo myrepo --path .
  1. Walk the working tree (respecting .gitignore) and group files by language.
  2. For each path, look up the git blob OID. If repo_marker says we already
     ingested that blob, skip.
  3. ast-grep extracts symbols (rust/ts/js/py/go only — see Cargo features).
  4. Upsert code_symbols + code_fts rows in one transaction per file.
  5. Update repo_marker.last_head = git rev-parse HEAD.

comemory ingest-code  (BYO embedder)
  • Reads JSONL rows from stdin of the shape
    `{"qualified": "...", "snippet": "...", "embedding": [..]}`.
  • Inserts into code_vec (dim guard) and refreshes the matching
    code_symbols / code_fts rows.
```

`comemory rebuild` drops `comemory.db` and reruns step 4 of "save" for every
markdown file. Use it after upgrading from v0.1 or after editing the DB by
hand.

## 8. Auto-update modes

Three configurable modes for keeping the code index fresh:

```toml
[indexing]
auto_reindex = "lazy"               # "lazy" | "hook" | "off"
auto_reindex_threshold_ms = 200
incremental_batch_size = 50
```

| Mode | Trigger | Behavior |
|---|---|---|
| `lazy` (default) | Before every `search` / `context` | Compare `git rev-parse HEAD` to `repo_marker.last_head`. If different and estimated cost is below the threshold, reindex incrementally in-line. Otherwise warn and proceed. |
| `hook` | git `post-commit`, `post-merge`, `post-checkout` | `comemory install-hooks` registers scripts that run `comemory index-code --incremental --quiet &`. |
| `off` | Manual only | `comemory index-code` runs only when invoked. |

`comemory doctor` always reports the staleness gap (commits behind HEAD)
for every known repo, regardless of mode.

## 9. Pruning

Three kinds of stale data, three detection paths, one command surface:

| Stale | Cause | Detection |
|---|---|---|
| Orphan SQL row | `.md` deleted but `memories` row remains | scan: id in DB ∧ id ∉ memories/ |
| Stale code symbol | source file deleted or content hash changed | re-`index-code`: file missing OR ast_hash mismatch |
| Low-value memory | quality + usage + irrelevance threshold | SQL join over `feedback` |

Soft delete moves `memories/{id}.md` → `memories/.trash/{id}.md`. Trash is
retained 30 days, then purged by `comemory gc`. SQL rows are hard-deleted
(always rebuildable from markdown).

`comemory index-code --incremental` auto-prunes code symbols for deleted
files. `comemory doctor` reports stale counts read-only, never deletes.

## Where to go next

- [CLI reference](cli-reference.md) — every command with worked examples.
- [v0.2 lightweight design spec](superpowers/specs/2026-06-07-lightweight-v2-design.md) — authoritative architecture and schema notes.
