# Architecture overview

This is a 2-page on-ramp into the comemory design — storage layout,
retrieval pipeline, save flow, and code-indexing flow. It is the
authoritative architecture reference; pair it with the
[CLI reference](cli-reference.md) for command-level detail.

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
                  │  │   router (candidates)      │     │
                  │  │     │  relaxed OR fallback │     │
                  │  │     ▼                      │     │
                  │  │   rerank (priors ×score)   │     │
                  │  │     │                      │     │
                  │  │     ▼                      │     │
                  │  │   diversify (MMR/SimHash)  │     │
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
| `store` | SQLite connection layer, schema_meta, migrations, vector + FTS helpers, identifier tokenizer (camelCase/snake_case split + FFI registration) |
| `simhash` | 64-bit SimHash + Hamming distance over tokenized memory bodies |
| `graph` | SQL-backed edges (`Supersedes`, `ConflictsWith`, `RelatesTo`, `ReferencesFile`, `ReferencesSymbol`, `CoChanged`, `Imports`, …) + recursive walks; `cross_link` parses backticked refs; `cochange` mines git history, `imports` extracts per-language import edges, `pagerank` + `materialize` write `code_symbols.rank_score` |
| `retrieval` | router (candidates + 4-tier lexical ladder ending in learned expansion), score (ACT-R/Beta primitives), rerank (multiplicative priors), diversify (SimHash collapse + MMR), pipeline (orchestration + access tracking), fuse (RRF), bundle (context lookup, code refs ranked by graph priors); code side: code_route (BM25 + thresholded ANN + RRF, chunk→parent coalesce), code_rerank + code_prior (PageRank / recency / working-set affinity / feedback) |
| `eval` | learning loop: golden sets (file + feedback harvest), recall@k/MRR metrics, eval runner (replays originating repo/kind filters), reformulation mining, grid tune |
| `ast` | ast-grep wrapper (rust/ts/js/py/go), per-language symbol extractor, cAST chunking of oversized symbols, user pattern API |
| `stats` | rusqlite usage / feedback / code_feedback / repo-marker tables (lives inside the same DB) |
| `config` | Layered config: built-in defaults → `config.toml` → env → CLI flags |
| `output` | TTY rendering (owo-colors) + JSON serializers (serde_json) |
| `prune` | Orphan, stale-code, low-value detection and (soft) deletion |
| `serve` | Loopback-only axum web server behind the `comemory serve` command (256-bit per-session token, Host-header guard, default-deny CORS, path-containment chokepoint) hosting the embedded React SPA: WebGL code-graph viewer + in-browser source editor with `If-Match` optimistic concurrency. The `comemory graph` command exports the same code graph as JSON / DOT / static HTML |
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
| `schema_meta` | Key/value rows: schema version, locked-in vector dimensions, code-format version, and migration markers |
| `memories` | Frontmatter + body mirror keyed by memory id |
| `memory_fts` (FTS5) | Lexical index over memory body + title |
| `memory_vec` (vec0) | Dense vectors keyed by memory id; dim locked at first save |
| `code_symbols` | Symbols extracted from indexed repos (file, kind, snippet, simhash) plus a materialized `rank_score` (PageRank) and `parent_id` (cAST chunk → parent symbol) |
| `code_fts` (FTS5) | Lexical index over symbol identifiers + snippets + path tokens |
| `code_vec` (vec0) | Dense vectors for code symbols; dim locked at first ingest |
| `edges` | Sparse weighted table replacing the kuzu graph (typed src→dst rows; includes mined `co_changed` + `imports` code-graph edges) |
| `retrieval_log`, `feedback`, `feedback_events`, `code_feedback`, `query_expansions`, `repo_marker` | Learning-loop telemetry (query log + per-query feedback provenance), aggregated memory + code-symbol feedback counters, mined expansions, indexing markers (incl. the v7 `repo_marker.root_path` working-tree root used by `serve` to resolve `file:<repo>:<path>` ids back to disk) |

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
"BYO-Vector workflow" section in the README). The dims (1024 for
`memory_vec`, 768 for `code_vec`) are baked into the vec0 DDL in
`src/store/sql/0002_v2_tables.sql` and are not env-configurable.
`COMEMORY_EMBED_HINT` records (and surfaces in `comemory doctor`) the
identifier of the embedder you used.

The `edges` table is a flat `(src_kind, src_id, edge_kind, dst_kind, dst_id)`
schema (plus an integer `weight`) that replaces the v0.1 kuzu graph for the
set of edges we actually use (`Supersedes`, `ConflictsWith`, `RelatesTo`,
`DerivedFrom`, `ReferencesFile`, `ReferencesSymbol`, `InRepo`, `AuthoredBy`,
`Tagged`, and the mined code-graph kinds `CoChanged` + `Imports`).
Multi-hop traversals use SQLite recursive CTEs.

## 5. Retrieval pipeline

The pipeline runs entirely in Rust. No LLM calls.

```
search("postgres migration race")
  │
  ├─ route  (router.rs)
  │   ├─ vector + non-empty query           → hybrid (ANN + FTS5 BM25, fused via RRF)
  │   ├─ vector + empty query               → pure vector (ANN only)
  │   └─ no vector                          → pure lexical (FTS5 BM25)
  │   ├─ --repo / --kind filters (when set) constrain every branch
  │   └─ lexical fallback ladder: when the strict lexical leg returns zero
  │       hits, retry word-OR (≥ 2 terms), then subtoken-OR, then a
  │       learned-expansion tier ORing in mined query_expansions mappings
  │       (never fires on the pure-vector path; hits carry a tier 1..4)
  │
  ├─ rerank  (rerank.rs)
  │   ├─ per-hit: ACT-R activation boost (recency × access count)
  │   ├─ Beta-smoothed feedback multiplier (used / irrelevant counts)
  │   ├─ quality multiplier (frontmatter quality 1-5)
  │   ├─ supersede penalty (fixed 0.2× if superseded by a live memory)
  │   └─ final_score = rrf × activation × feedback × quality × supersede
  │       (activation/feedback/quality clamped to [prior_clamp.lo, prior_clamp.hi];
  │        the supersede penalty intentionally bypasses the clamp)
  │
  ├─ diversify  (diversify.rs)
  │   ├─ SimHash near-dup collapse (Hamming ≤ threshold → keep highest score)
  │   └─ MMR re-ranking (mmr_lambda blends relevance vs. diversity)
  │
  └─ emit  (output/search.rs)
      ├─ TTY: one line per hit with colored score + source label
      └─ JSON: {"hits":[{"memory_id","score","source","tier","superseded_by"?,"score_parts":{
               rrf, activation, feedback, quality, supersede, final_score}}],"query_id"?}
```

`score_parts` is a stable explainability contract (`comemory tune` reads
it); its `rrf` field is the max-normalized relevance in `[0, 1]` (pool max
maps to 1.0), not the raw fused score.
Identifier-aware matching (camelCase/snake_case splitting) is not a routing
branch — the custom `identifier` FTS5 tokenizer is baked into the
`memory_fts` / `code_fts` DDL, so every lexical query benefits from it.

`comemory search-code` runs a parallel code-side pipeline: `code_route`
(weighted BM25 over symbol/snippet/path_tokens + an optional thresholded
BYO-vector ANN leg, fused via RRF; chunk hits coalesce to their parent
symbol) followed by `code_rerank`, which multiplies the relevance by four
priors from `code_prior` — materialized PageRank, recency, working-set
affinity (dirty/recent files in the current checkout), and Beta-smoothed
`code_feedback`. Hits carry a `score_parts` breakdown and the envelope a
`query_id` for `comemory feedback --used-code`. `comemory context` ranks
the code refs in its bundle with the same graph priors.

## 6. Save flow

```
comemory save "..." --kind=decision [--vector ... | --vector-stdin]
  1. Parse args; build Memory; assign id = sha256(body)[:8].
  2. Validate vector dim (if supplied) against schema_meta — fails fast.
  2a. Near-duplicate check (best-effort): scan live memories rows via SimHash
      Hamming distance. If a near-dup is found, record duplicate_of id.
      TTY: stderr warning. JSON: duplicate_of field. Save always proceeds.
  3. Atomic markdown write: memories/.{id}.tmp → memories/{id}-{slug}.md.
  4. SQLite upsert (inside one transaction):
       - memories row (+ simhash column)
       - memory_fts row
       - memory_vec row (only if a vector was supplied)
       - edges from cross_link::extract_refs (ReferencesFile / ReferencesSymbol)
  5. git add + commit + push (best-effort, only when COMEMORY_GIT_AUTO_SYNC is on).
```

Markdown is always the source of truth. If the SQLite mirror transaction
fails, the markdown file is **kept** (it was already written as the source
of truth) and the error names the markdown path with a hint to run
`comemory rebuild`, which reconstructs the DB from `memories/*.md`.

## 7. Code indexing flow

```
comemory index-code --repo myrepo --path .
  1. Walk the working tree (respecting .gitignore) and group files by language.
  2. For each path, look up the git blob OID. If repo_marker says we already
     ingested that blob, skip.
  3. ast-grep extracts symbols (rust/ts/js/py/go only — see Cargo features).
     Oversized symbols are split into child chunk rows at AST boundaries
     (cAST); chunks point at their parent via code_symbols.parent_id.
  4. Upsert code_symbols + code_fts rows in one transaction per file.
  5. Mine the code graph: co_changed edges from git history (windowed, with
     a mega-commit guard and a last_mined_commit cursor) and imports edges
     from per-language import resolution.
  6. Run weighted PageRank over the graph and materialize the score into
     code_symbols.rank_score (read by search-code / context reranking).
  7. Update repo_marker.last_head = git rev-parse HEAD.

comemory ingest-code  (BYO embedder)
  • Reads JSONL rows from stdin of the shape
    `{"qualified": "...", "snippet": "...", "embedding": [..]}`.
  • Inserts into code_vec (dim guard) and refreshes the matching
    code_symbols / code_fts rows.
```

**Deleted-files gap (known limitation):** `index-code` only walks files
that exist in the working tree, so symbols, `indexed_files` cursors, and
`co_changed`/`imports` edges for files *deleted* from a repo persist in
the index until a future stale-code prune lands (an M4 candidate — see
`src/prune/stale_code.rs`). Until then, deleted files keep their PageRank
mass and can still surface in `search-code` results.

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
| `lazy` (default) | `search-code` / `context`, when the repo's HEAD moved | A **detached, non-blocking** background `comemory index-code --repo <repo> --path <root>` is spawned, then the search proceeds against the *current* (possibly slightly stale) index immediately — the read path never blocks on or fails because of the reindex. See [§8.1](#81-lazy-auto-reindex). |
| `hook` | git `post-commit`, `post-merge`, `post-checkout` | `comemory install-hooks` registers scripts that run `comemory index-code --repo <repo> --path <root> &`. No in-process trigger. |
| `off` | Manual only | `comemory index-code` runs only when invoked. |

### 8.1 Lazy auto-reindex

`lazy` is wired in `src/cli/lazy_reindex.rs`, shared by `search-code` and
`context` (Binding Rule 1). A trigger fires only when **all** hold: mode is
`lazy`, the command runs inside a git repo (CWD discovery, same policy as the
working-set affinity probe), and the code index is **stale**.

- **Staleness probe (cheap — runs on every search).** Stale iff the repo was
  never indexed (no `repo_marker` row, or a NULL `last_mined_commit`) OR the
  current repo HEAD differs from `repo_marker.last_mined_commit` (the cursor
  `graph::materialize` advances to HEAD after each successful `index-code`).
  Cost: one `git2` HEAD resolve plus two single-row SQLite reads — **no**
  working-tree walk or per-file blob hash. Consequently, uncommitted
  (un-HEAD) working-tree edits are **intentionally not** detected by the lazy
  probe; the git `hook` mode or a manual `index-code` covers those. Lazy is a
  best-effort freshness fallback keyed on HEAD.
- **Detached spawn.** `std::process::Command` on `std::env::current_exe()`
  runs `index-code --repo <repo> --path <root> --data-dir <dir>` with null
  stdio; the child handle is dropped (not awaited). Best-effort: a missing
  `current_exe` or a spawn error is logged via `tracing` and swallowed — a
  failed spawn never surfaces as a search error.
- **Debounce.** The last trigger is recorded in `schema_meta` under
  `lazy_reindex_head:<repo>` as `"<head>|<unix_millis>"`. A fresh spawn is
  suppressed when EITHER the recorded head equals the current head (a reindex
  already fired for this HEAD), OR the recorded trigger is younger than
  `auto_reindex_threshold_ms` (the herd guard against a burst of searches).
- **`incremental_batch_size`** is currently **reserved** (parsed/validated,
  not consumed): `index-code` runs its whole walk in one transaction with no
  batch seam to thread it through, and the lazy trigger delegates to that
  single invocation. It is kept as an honest reserved knob for a future
  chunked-commit indexing path rather than wired to a fake consumer.

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

Stale-code pruning is **not implemented yet** (`prune::stale_code::detect`
is a stub returning an empty set): code symbols and graph edges for files
deleted from a repo persist — keeping their PageRank mass — until a future
stale-code prune lands (M4 candidate).

## Where to go next

- [CLI reference](cli-reference.md) — every command with worked examples.
- [README](../README.md) — install, quickstart, and the feature tour.
