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
| `store` | SQLite connection layer, schema_meta, migrations, vector + FTS helpers, identifier tokenizer (camelCase/snake_case split + FFI registration), `edge_fts` (the triplet index over `edges` — rendering, refresh, and the ladder behind `comemory edges`) |
| `simhash` | 64-bit SimHash + Hamming distance over tokenized memory bodies |
| `graph` | SQL-backed edges (`Supersedes`, `ConflictsWith`, `RelatesTo`, `ReferencesFile`, `ReferencesSymbol`, `CoChanged`, `Imports`, …) + recursive walks; `cross_link` parses backticked refs; `cochange` mines git history, `imports` extracts per-language import edges, `pagerank` + `materialize` write `code_symbols.rank_score`, `memory_rank` writes `memories.rank_score` from the derived memory graph, `derived` refreshes every derived artifact in one best-effort post-write pass (§5.3) |
| `retrieval` | router (candidates + 4-tier lexical ladder ending in learned expansion), graph_route (graph-expansion leg: an edge walk seeded from the provisional top hits), scope (the created-date `TimeScope` + the `Filters` bundle threading repo/kind/time through every leg), score (ACT-R/Beta primitives + the shared median/PageRank-boost math), rerank (five multiplicative priors, including the memory PageRank boost), diversify (SimHash collapse + MMR), pipeline (orchestration + access tracking), fuse (RRF, pairwise + N-ary), bundle (context lookup, code refs ranked by graph priors); code side: code_route (BM25 + thresholded ANN + RRF, chunk→parent coalesce), code_rerank + code_prior (PageRank / recency / working-set affinity / feedback) |
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

Markdown is the single source of truth. `comemory.db` is a rebuildable
mirror: `comemory rebuild` reconstructs the memory layer (rows, `memory_fts`,
edges) from `memories/*.md`, and carries everything markdown cannot rebuild
— the code index, the document index, and the learning-loop tables — across
from the pre-rebuild database rather than re-walking indexed repos (§6). It
also snapshots the still-live database before the swap; see §3.2 for the
snapshot mechanism it shares with schema migration.

### 3.1 Inside `comemory.db`

One SQLite file replaces v0.1's `lancedb/`, `kuzu/`, and `stats.db` trio.
The database is created on first use, extended with the `sqlite-vec`
extension at runtime, and version-tracked through `schema_meta` so future
migrations stay idempotent.

| Table | Purpose |
|---|---|
| `schema_meta` | Key/value rows: schema version, locked-in vector dimensions, code-format version, and migration markers |
| `memories` | Frontmatter + body mirror keyed by memory id, plus a materialized `rank_score` (memory-graph PageRank) |
| `memory_fts` (FTS5) | Lexical index over memory body + title |
| `memory_vec` (vec0) | Dense vectors keyed by memory id; dim locked at first save |
| `code_symbols` | Symbols extracted from indexed repos (file, kind, snippet, simhash) plus a materialized `rank_score` (PageRank) and `parent_id` (cAST chunk → parent symbol) |
| `code_fts` (FTS5) | Lexical index over symbol identifiers + snippets + path tokens |
| `code_vec` (vec0) | Dense vectors for code symbols; dim locked at first ingest |
| `edges` | Sparse weighted table replacing the kuzu graph (typed src→dst rows; includes mined `co_changed` + `imports` code-graph edges) |
| `edge_fts` (FTS5) | Derived triplet index over `edges`: each row rendered as searchable `src —rel→ dst` text with the raw edge carried in UNINDEXED payload columns. Refresh-materialized (§5.3), never written incrementally |
| `retrieval_log`, `feedback`, `feedback_events`, `code_feedback`, `query_expansions`, `repo_marker` | Learning-loop telemetry (query log + per-query feedback provenance), aggregated memory + code-symbol feedback counters, mined expansions, indexing markers (incl. the v7 `repo_marker.root_path` working-tree root used by `serve` to resolve `file:<repo>:<path>` ids back to disk) |

Every dense lookup goes through `sqlite-vec`'s `vec0` virtual table with a
dimension guard so a mismatched embedder fails fast (`VecDimMismatch`)
instead of corrupting the index. FTS5 hits and vector hits are fused via
Reciprocal Rank Fusion (RRF, `k = 60` by default).

### 3.2 Schema migration & upgrade safety

`store::migrate::run` has exactly one production call site,
`store::connection::open`, so an upgraded binary migrates a user's
`comemory.db` automatically on the very next command — there is no
`comemory migrate` command and no way to skip it. The chain itself is a
single source of truth, `store::migrate::list::MIGRATIONS`: each entry
carries its key, SQL, a `Class` (`Additive` or `Destructive`, verified
against the migration SQL rather than hand-labeled), an optional post-apply
Rust pass (the two simhash backfills), and the `schema_meta` marker keys it
writes.

Immediately before the chain runs, `store::migrate::preflight` guards two
things:

- **Forward-compat refusal.** It compares the `schema_meta` marker keys the
  database has applied against the set every migration in `MIGRATIONS` is
  known to write. A key outside that set means the database was written by
  a *newer* comemory; the open is refused with `Error::SchemaTooNew` (exit
  `70`) naming the unknown key, and `schema_meta` is left untouched. Every
  command except `doctor` surfaces that refusal as-is; `doctor` falls back
  to a second, read-only connection and reports the unknown keys as the
  `unknown_migration_keys` field instead, since explaining a broken state
  is its job. A genuinely broken migration (its SQL fails to apply, or a
  *mandatory* pre-upgrade snapshot fails ahead of a `Destructive`
  migration) surfaces as `Error::Migration` instead, and `doctor` propagates
  that like every other command — the fallback is scoped to the
  forward-compat refusal alone.
- **Pre-upgrade snapshot.** Whenever any migration is pending — additive or
  destructive — `store::migrate::backup::snapshot` `VACUUM INTO`s the
  database to `comemory.db.pre-v{N}.bak` (`{N}` = the version being left)
  before the chain touches it — `VACUUM INTO` is used instead of a raw file
  copy because it captures committed-but-not-yet-checkpointed WAL frames
  that a bare copy would miss. Only the *consequence of a snapshot failure*
  depends on destructiveness, not whether the snapshot is attempted:
  mandatory (a failure refuses the upgrade) ahead of a `Destructive`
  migration, advisory (a failure only warns) when every pending migration is
  `Additive`; either way the snapshot attempt itself is skippable via
  `COMEMORY_SKIP_MIGRATION_BACKUP=1`. The newest two snapshots per database
  file are kept, pruned before each new one is taken; an existing `.bak` is
  `PRAGMA quick_check`ed before being trusted, so a truncated file left by a
  killed process is replaced rather than relied on. `comemory rebuild`
  reuses the same snapshot mechanism — `comemory.db.pre-rebuild.bak` —
  immediately before its atomic swap.

`store::migrate::set_version` additionally refuses to write a version lower
than the one already stored (compared numerically, not lexically), so an
older binary opening a newer database cannot stomp `schema_meta.version`
downward even if it no-ops through every migration it recognizes.

`scripts/migration-check.sh` closes the loop at the source level: every
already-released `src/store/sql/*.sql` file must stay byte-identical to its
content at the first release tag that shipped it, because the runner is
marker-keyed and idempotent — editing a shipped migration changes what a
live user's database already applied without anything re-running it. New
schema changes are always a new, appended, numbered file.

See [Upgrading comemory](guides/upgrading.md) for the user-facing walkthrough
— including how to restore a snapshot and the `comemory serve` restart
caveat.

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
  ├─ graph expand  (graph_route.rs)          — third candidate leg
  │   ├─ seeds = top graph_seeds hits of the provisional ranking above
  │   ├─ one recursive CTE over edges, depth ≤ graph_hops, traversed
  │   │   undirected (both orientations) over an allowlist of rels;
  │   │   the hub rels in_repo / authored_by / tagged are excluded
  │   ├─ MIN(depth) per memory, live rows only, --repo/--kind applied,
  │   │   ranked (hops ASC, memory_id ASC)
  │   └─ fused in as one more RRF list (fuse::rrf_multi); ids only the
  │       walk found are labeled source "graph", tier 0
  │
  ├─ rerank  (rerank.rs)
  │   ├─ per-hit: ACT-R activation boost (recency × access count)
  │   ├─ Beta-smoothed feedback multiplier (used / irrelevant counts)
  │   ├─ quality multiplier (frontmatter quality 1-5)
  │   ├─ supersede penalty (fixed 0.2× if superseded by a live memory)
  │   ├─ PageRank boost from memories.rank_score, relative to the median of
  │   │   the pool's DISTINCT scores: 1 + 0.2·ln(1 + raw/median)
  │   └─ final_score = rrf × activation × feedback × quality × supersede × rank
  │       (activation/feedback/quality/rank clamped to [prior_clamp.lo, prior_clamp.hi];
  │        the supersede penalty intentionally bypasses the clamp)
  │
  ├─ diversify  (diversify.rs)
  │   ├─ SimHash near-dup collapse (Hamming ≤ threshold → keep highest score)
  │   └─ MMR re-ranking (mmr_lambda blends relevance vs. diversity)
  │
  └─ emit  (output/search.rs)
      ├─ TTY: one line per hit with colored score + source label
      └─ JSON: {"hits":[{"memory_id","score","source","tier","superseded_by"?,"score_parts":{
               rrf, activation, feedback, quality, supersede, rank, final_score}}],"query_id"?}
```

`score_parts` is a stable explainability contract (`comemory tune` reads
it); its `rrf` field is the max-normalized relevance in `[0, 1]` (pool max
maps to 1.0), not the raw fused score.

The fifth prior, `rank`, is the memory-side mirror of what `code_prior`
already does for symbols. `graph::memory_rank` runs PageRank over a graph
derived at compute time — the direct memory→memory relations
(`supersedes`, `conflicts_with`, `derived_from`, `relates_to`) plus
undirected co-citation edges between memories that reference the same file
or symbol, with the hub rels `in_repo` / `authored_by` / `tagged` excluded
for the same reason the graph walk excludes them — and writes the result to
`memories.rank_score`. Because absolute PageRank scales with `1/n`, the
boost is taken relative to the median of the candidate pool's *distinct*
scores, which makes it corpus-size invariant. The recompute is a
best-effort post-pass, one of the two derived artifacts refreshed together
at the seams described in §5.3.

Two regimes make the prior neutral, both order-preserving. A corpus where
no pass has ever run leaves every `rank_score` at the `0.0` column default,
so the pool median is 0 and the boost is exactly `1.0`. A corpus that has
been ranked but carries no qualifying edges gets uniform PageRank (`1/n`
each), so every candidate's raw score equals the median and every boost is
the same `1 + 0.2·ln 2 ≈ 1.1386`. A uniform multiplier cannot reorder a
ranking, which is why upgrading an existing database — and the committed
ranking snapshots — sees no movement until real structure exists.
Identifier-aware matching (camelCase/snake_case splitting) is not a routing
branch — the custom `identifier` FTS5 tokenizer is baked into the
`memory_fts` / `code_fts` DDL, so every lexical query benefits from it.

The graph-expansion leg exists because a memory can be *lexically dark* for
a query — different vocabulary, no shared subtokens — while `edges` already
links it to a top hit. Expansion runs **after** leg ranking rather than as a
per-hop re-query: whichever routing branch fired produces a provisional
ranking, its top `graph_seeds` ids (default 8) seed one walk, and the result
fuses back in. The trade-off is that graph candidates cannot themselves seed
further expansion — that is what `graph_hops` (default 2) covers. Ranking
inside the leg is `(hops, memory_id)` only; edge weights are not consulted.
The leg is strictly additive: `COMEMORY_RETRIEVAL_GRAPH_HOPS=0` and an empty
expansion both return the provisional ranking through the same functions
that produced it, so the disabled path is the pre-leg pipeline by
construction (pinned by a committed parity snapshot). `rerank`, `diversify`
and pagination are source-agnostic, so a graph candidate is scored, deduped
and penalized exactly like any other — a superseded one still takes the
0.2× supersede penalty.

`comemory search-code` runs a parallel code-side pipeline: `code_route`
(weighted BM25 over symbol/snippet/path_tokens + an optional thresholded
BYO-vector ANN leg, fused via RRF; chunk hits coalesce to their parent
symbol) followed by `code_rerank`, which multiplies the relevance by four
priors from `code_prior` — materialized PageRank, recency, working-set
affinity (dirty/recent files in the current checkout), and Beta-smoothed
`code_feedback`. Hits carry a `score_parts` breakdown and the envelope a
`query_id` for `comemory feedback --used-code`. `comemory context` ranks
the code refs in its bundle with the same graph priors.

### 5.1 Bounded-window pagination

The data-returning retrieval commands (`search`, `search-code`, `context`)
page within a **bounded window**. The pipeline fetches a candidate pool
sized `clamp(offset + limit + limit, CANDIDATE_POOL=50, max_page_window)`
(env `COMEMORY_RETRIEVAL_MAX_PAGE_WINDOW`, default `200`), runs the full
fuse → rerank → diversify over that pool, then slices `[offset,
offset+limit]`. Because the RRF/MMR top-prefix is stable as the pool grows,
pages don't drift as you walk deeper. `has_more` is forced **false** once
the window ceiling is reached — deeper results require a refined query, not
more paging.

The `--json` envelope is `Page<T>` = `{items, limit, offset, total,
has_more}`. For retrieval, `total` is the **in-window** ranked count, not a
global match count.

The mechanism differs per command: `list` / `graph` page via SQLite
`LIMIT/OFFSET`; `ast` / `prune` page **in-memory** (and `prune` paging is
display-only — `--apply` acts on the full set, not the current page).

### 5.2 Time scoping (`--since` / `--until` / `--as-of`)

`search` and `context` accept a created-date window, carried as a
`retrieval::scope::TimeScope` inside the `Filters` bundle that already
holds `--repo` / `--kind`. All three candidate legs apply it to
`memories.created_at`: the lexical ladder (through the single
`store::fts_memory::run_memory_match` choke-point, so every tier is
covered), the vector KNN, and the graph walk. Bounds travel as normalized
ISO-8601 strings and every predicate compares through SQLite
`datetime(…)` rather than raw string `<=`: stored precision is mixed (the
writer emits 9-digit fractional seconds, older rows and seeds second
precision) and lexicographic compare inverts across precisions. Two
consequences fall out of `datetime()`: it truncates fractional seconds, so
`--until <date>` behaves as `<= 23:59:59` of that day and sub-second bound
precision is ignored (the inclusive-window intent is unaffected); and a row
whose `created_at` will not parse yields NULL and drops out of any scoped
run.

`--as-of` is `--until` plus **supersede scoping**: the rerank stage only
counts a superseder that itself existed at the cutoff, so a memory
superseded later shows its original, unpenalized score. That check keys on
the superseder *memory's* `created_at` — never on `edges.created_at`, which
`comemory rebuild` re-stamps with the rebuild time, while frontmatter
`created` survives it. When several live superseders exist the earliest one
wins (`ORDER BY datetime(created_at), id`), so the reported `superseded_by`
is deterministic in and out of as-of mode.

Time scoping is deliberately **corpus-only**. The ACT-R activation prior
still uses wall-clock `now` and present-day access counts (access history
is not versioned, so "activation as it was then" is unknowable), and
soft-deleted memories stay excluded even when the cutoff predates the
deletion — their FTS/vec rows and edges are physically purged at
soft-delete. An unscoped run binds NULL for both bounds, which every
predicate short-circuits on, so the no-flags path is bit-identical to the
pre-time-travel pipeline.

### 5.3 Derived artifacts and their refresh seams

Two things in `comemory.db` are computed *from* `memories` + `edges` rather
than written alongside them: the memory-graph PageRank in
`memories.rank_score` (§5) and the `edge_fts` triplet index that backs
`comemory edges`. They share a staleness window and a set of trigger points,
so they share one entry point — `graph::derived::refresh_derived_best_effort`
— and a new seam cannot refresh half the derived state. Each artifact is
independently best-effort: a failure warns and the other still runs.

Four seams call it, each only after its own transaction has committed, so a
failed refresh costs freshness and never the primary write:

- `save`, after the SQLite mirror commits;
- soft delete, inside `mirror_soft_delete`, so `comemory delete` and both of
  `comemory prune`'s delete paths behave alike;
- `rebuild`, after the markdown replay and the preserved-table copy, before
  the atomic swap;
- `index-code`, after `graph::materialize` returns. This seam is new, and it
  closes a real gap: `materialize` writes the `co_activated` memory→file
  edges earned by the co-activation reward (§7.1), but nothing used to
  recompute rank afterwards, so a reward sat in `edges` unread until the
  next save.

`edge_fts` is **refresh-materialized, not written through**: one transaction
deletes the table and re-inserts it from `edges` in a single ordered
`INSERT … SELECT`. The alternative — hooking each of the ten-plus edge write
paths, two of which are raw `DELETE FROM edges` statements inside
`materialize` — is incremental but rots the moment the next edge writer
forgets a hook. At personal-memory scale a full pass is milliseconds, the
same economics that make `memory_rank` a full recompute. Insertion order is
pinned to `(src_kind, src_id, rel, dst_kind, dst_id)`, so two databases
holding the same edges index identically.

Migration 0012 creates `edge_fts` **empty** on purpose: the triplet
rendering lives in Rust and only there, and duplicating it as migration SQL
would be a drift trap. An upgraded database therefore arrives with edges but
no triplets, and heals itself — the first `comemory edges` invocation sees
an empty index over a non-empty `edges`, refreshes once, and answers in the
same run. No flag, no backfill, no separate upgrade step.

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
`comemory rebuild`, which reconstructs the memory layer from `memories/*.md`
and — via a pre-swap snapshot plus an `ATTACH`-based copy — preserves
everything else already in `comemory.db` (§3.2, §7).

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

### 7.1 Auto-reinforcement reward

`index-code` harvests an implicit-feedback signal on every run — always on,
no flag. When `graph::materialize` mines commit co-activation, it applies a
**triple-channel** reinforcement reward for each `(memory, referenced-file)`
pair that co-occurs in a commit:

1. a weighted `co_activated` memory→file edge (accumulating weight by the
   file's per-pass touch count);
2. an ACT-R activation bump (`memories.access_count += 1`, plus
   `last_accessed`);
3. a confidence-gated Bayesian `used` increment that fires **once**, the
   first time the edge weight crosses the threshold (`≥ 2`), writing a
   `feedback_events` row under a sentinel query id — excluded from
   golden-set harvest. Provenance is classified at mint time:
   - `auto_search_edit` / `auto-search-edit` when the memory also appeared
     in a recent `retrieval_log` page (`source` = `search` or `context`,
     within `reinforce.search_edit_days`, default 7; unscoped
     `repo IS NULL` rows still credit);
   - otherwise `auto_coactivation` / `auto-coactivation`.

The whole reward runs inside `materialize`'s transaction and is idempotent
via the `repo_marker.last_mined_commit` cursor (read before, advanced after,
in the same transaction), so reruns over already-mined commits make every
delta zero and never double-count. Migration `0008` adds the `co_activated`
edge kind and the `feedback_events.provenance` column; `0010` adds
`bandit_arms` for the eval-gated online bandit (`comemory bandit`).
`comemory rebuild` preserves this earned state rather than discarding it.

**Phase 0:** `comemory context` bumps the code-ref `access_count` for its
resolved refs (parity with `search-code`'s existing access tracking), so the
recency/activation priors stay honest across both read paths.

`comemory rebuild` builds a fresh `comemory.db.rebuild.tmp`, replays step 4
of "save" for every markdown file into it, copies in everything markdown
cannot rebuild (§6), snapshots the live DB, then atomically swaps the tmp
file over it. Use it after the DB is corrupted, deleted, or edited by hand.

### 7.2 Versioned-pointer code references

`comemory save --ref-file <[repo:]path>` and `--ref-symbol <[repo:]path:symbol>`
attach **explicit, version-pinned** links from a memory to code. At save time, if
the referenced path is tracked in the current git repo, comemory captures a
**versioned anchor** — the file's HEAD-tree blob OID, the HEAD commit SHA, and the
branch — and records it in the markdown frontmatter (`references.{files,symbols}`,
the source of truth). There is no content snapshot: refs point at *live* code, the
anchor only records *which committed state* the link was made against.

These refs are materialized two ways by `memory_row::insert` (so `comemory rebuild`
restores them from markdown for free): a `references_file` / `references_symbol`
row in the `edges` table (graph shape) **plus** a row in the dedicated `code_ref`
side table carrying the anchor (`pinned_blob`, `pinned_commit`, `branch`). The
side table is a full-replace on re-save, so a dropped ref is actually removed.

**Staleness model.** On `comemory context`, each ref is classified by comparing
its `pinned_blob` against the file's *current* HEAD-tree blob — a cheap git
lookup, no reindex:

- `fresh` — pinned blob equals the current HEAD blob.
- `stale` — the committed file changed since the anchor was taken.
- `ghost` — the target no longer exists: the file is gone from HEAD, or (for a
  symbol) the file is present but the symbol is absent from a **current** code
  index.
- `unpinned` — no anchor was captured (untracked path, unborn HEAD, cross-repo,
  or a body-mined backtick mention).
- `unknown` — pinned but unverifiable now (repo not on disk, or a symbol-ghost
  verdict needs an index that is absent or stale).

File refs are **index-independent** (fresh/stale/ghost come straight from the
git blob compare); only the symbol-*ghost* verdict is **index-dependent**, and
when no current index covers the file it degrades to `unknown` rather than a
false `ghost`. A `ghost` symbol ref also makes its owning memory eligible for the
`comemory prune` ghost-ref rule.

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

## 10. Interactive terminal explorer (`comemory tui`)

`comemory tui` is a read-only, terminal-native front end over the same
retrieval pipeline the one-shot commands use — it adds no ranking of its own.
A ratatui UI runs an async `EventStream` + `tokio::select!` loop; as-you-type
queries are debounced and run lexically (FTS5, no embedder), and an explicit
`Ctrl-S` on the Memory tab shells out to a configured embed command for
semantic enrichment.

Because `store::connection::open` hands back one non-`Sync` connection, a
dedicated worker thread owns it and answers requests over channels; each
response carries the request's generation `seq` so the loop can discard the
results of a superseded query. Every query sets `track = false`, so browsing
never writes `retrieval_log` or bumps `access_count`. The UI draws to stderr
and reserves stdout for the Enter-selected id (`id=$(comemory tui)`); the
terminal is restored by an RAII guard on every exit path. It complements — it
does not replace — `comemory serve`'s in-browser viewer.

## Where to go next

- [CLI reference](cli-reference.md) — every command with worked examples.
- [Upgrading comemory](guides/upgrading.md) — the schema-migration snapshot
  and forward-compat guard (§3.2), user-facing.
- [README](../README.md) — install, quickstart, and the feature tour.
