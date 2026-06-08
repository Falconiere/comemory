# comemory v0.2 — Lightweight "BYO-Vector + One SQLite" Design

**Status:** Draft for review
**Date:** 2026-06-07
**Author:** Falconiere Barbosa
**Drives:** Refactor to cut release binary from ~117 MB to ~25 MB while
preserving the product vision (agentic RAG over markdown memories + code).

---

## 1. Motivation

The current `release` binary measures **116.8 MB**. The weight comes almost
entirely from three embedded engines that comemory pulls in transitively:

| Component | Final-binary share | Reason it is heavy |
|-----------|--------------------|---------------------|
| `fastembed` + `ort` + `ort-sys` + `tokenizers` + `image` + `hf-hub` | ~30 MB | Static `onnxruntime`, image-processing code unused for text embedders, `hf-hub` model downloader, OpenSSL via `native-tls`. |
| `lancedb` + `lance*` + `datafusion*` + `arrow-*` + `sqlparser` | ~25 MB | A columnar vector store backed by a full SQL execution engine for use cases (joins, planning, aggregates) that comemory never exercises. |
| `kuzu` | ~25 MB | An embedded property-graph DB. comemory uses a handful of node/edge tables that fit trivially in SQL. |

Add to that the `axum` stack for `comemory serve` (~5 MB), the
`ast-grep-language` default-features fan-out (~10 MB across 22
tree-sitter parsers), and ~11 MB of unwinding tables that fall out the
moment `panic = "abort"` is set.

This document specifies a v0.2 refactor that:

1. Stops running an in-process embedder. The **caller** (an agent, a
   wrapper shell script around Ollama, or any HTTP embedding service)
   produces vectors and hands them to comemory.
2. Collapses memory storage, vector storage, lexical storage, code-symbol
   storage, graph storage, and stats into **a single SQLite database**
   (`~/.comemory/comemory.db`), with [`sqlite-vec`](https://github.com/asg017/sqlite-vec)
   vendored and statically linked for approximate-nearest-neighbor search.
3. Drops `comemory serve` and prunes the ast-grep language matrix to the
   five languages comemory actually targets (rust, typescript, javascript,
   python, go).
4. Keeps markdown the source of truth — the SQLite file is a derived,
   rebuildable cache.

Projected binary after the refactor: **20–30 MB**.

## 2. Non-Goals

- This refactor does **not** change the product surface: `save`, `search`,
  `context`, `index-code`, `ast`, `list`, `show`, `edit`, `delete`,
  `prune`, `doctor`, `stats`, `feedback` all remain.
- We do not ship a built-in embedding model. Embedding is the caller's
  responsibility; comemory provides a documented BYO-vector contract and
  a sample wrapper script.
- We do not migrate existing data. Project is pre-1.0; the v0.1 directory
  layout (`lancedb/`, `kuzu/`) is dropped without conversion. The
  `memories/*.md` corpus is the only thing that survives — `comemory
  rebuild` re-derives the SQLite database from it.
- No backwards-compatibility shims for v0.1 flags, env vars, or DB paths.

## 3. Architecture

### 3.1 Dependency swap

| Was | Becomes | Reason |
|-----|---------|--------|
| `fastembed`, `ort`, `ort-sys`, `tokenizers`, `image`, `ndarray`, `hf-hub`, `rayon` | _(removed)_ | Embedding is delegated to the caller. |
| `kuzu` | `rusqlite` (already in deps) | Edges become a relational table; graph walks become recursive CTEs. |
| `lancedb`, `lance*`, `datafusion*`, `sqlparser`, `arrow-array`, `arrow-schema` | `rusqlite` + vendored `sqlite-vec` (~150 KB C source) | ANN lives in `sqlite-vec`; lexical lives in SQLite FTS5 (bundled). |
| `axum`, `tower`, `tower-http`, `mime_guess`, `rust-embed`, `open` | _(removed, drop `serve`)_ | The web UI is out of scope for v0.2. |
| `ast-grep-language` (default 22 tree-sitter parsers) | `ast-grep-language` with `default-features = false` and only `tree-sitter-rust`, `tree-sitter-typescript`, `tree-sitter-javascript`, `tree-sitter-python`, `tree-sitter-go` | The four-language working set covers comemory's targets. |

**New deps**

- `sqlite-vec` — C source vendored, compiled via `build.rs`, statically
  linked into the bundled SQLite that rusqlite ships. Loaded at
  connection-open time through `Connection::load_extension`.
- `cc` (build-dependency) — to compile the sqlite-vec object.

**Kept (subset)**

`rusqlite` (now with `load_extension`, `vtab`, `blob` features in addition
to `bundled`), `ast-grep-core`, `git2` (already vendored libgit2),
`clap`, `serde*`, `tracing`, `time`, `regex`, `walkdir`, `ignore`,
`sha2`, `siphasher`, `owo-colors`, `thiserror`.

### 3.2 Top-level data flow

```
                    ┌───────────────────────────────────┐
                    │  caller (agent / wrapper script)  │
                    │  computes vectors via Ollama,     │
                    │  OpenAI, or any HTTP service      │
                    └───────────────┬───────────────────┘
                                    │  vector JSON
        ┌───────────────────────────┼───────────────────────────┐
        │                           │                           │
        ▼                           ▼                           ▼
   comemory save             comemory search             comemory index-code
   (md + frontmatter)        (ANN | FTS5 | hybrid)       (ast-grep walk)
        │                           │                           │
        └────────────┬──────────────┴───────────────┬───────────┘
                     ▼                              ▼
          markdown source of truth          ~/.comemory/comemory.db
          (~/.comemory/memories/*.md)       ┌──────────────────────┐
                                            │ memories             │
                                            │ memory_tags          │
                                            │ memory_vec  (vec0)   │
                                            │ memory_fts  (fts5)   │
                                            │ code_symbols         │
                                            │ code_vec    (vec0)   │
                                            │ code_fts    (fts5)   │
                                            │ indexed_files        │
                                            │ edges                │
                                            │ search_stats         │
                                            │ feedback             │
                                            │ schema_meta          │
                                            └──────────────────────┘
```

Markdown remains the source of truth. The SQLite database is rebuildable
end-to-end from `memories/*.md` plus a re-walk of any indexed repos —
`comemory rebuild` is a safe escape hatch.

### 3.3 Projected binary size

| Profile | v0.1 measured | v0.2 projected |
|---------|---------------|----------------|
| `release` (`lto="fat"`, `codegen-units=1`, `strip="symbols"`, plus `panic="abort"`) | 117 MB | 20–30 MB |
| `release-quick` (thin LTO) | 236 MB | ~60 MB |

Breakdown of the projected ~90 MB savings: ort + onnxruntime ~30 MB,
kuzu ~25 MB, lance + datafusion stack ~25 MB, ast-grep language trim
~10 MB, axum stack ~5 MB, `panic = "abort"` (eh_frame +
gcc_except_tab) ~11 MB, miscellaneous dead-code stripping after LTO
~5 MB. The minimum target is conservative; the actual figure will be
re-measured during implementation.

## 4. Storage Schema

Single file: `~/.comemory/comemory.db` (override `COMEMORY_DATA_DIR`).

- `PRAGMA journal_mode = WAL`
- `PRAGMA busy_timeout = 5000`
- `PRAGMA foreign_keys = ON`
- `sqlite-vec` extension is loaded on every `store::connection::open`.

### 4.1 Tables

```sql
-- ─── memories ────────────────────────────────────────────────────────────
CREATE TABLE memories (
    id            TEXT    PRIMARY KEY,            -- 8-hex prefix sha256(body)
    slug          TEXT    NOT NULL,
    kind          TEXT    NOT NULL CHECK (kind IN
                          ('decision','bug','convention',
                           'discovery','pattern','note')),
    repo          TEXT,
    author        TEXT,
    quality       INTEGER NOT NULL DEFAULT 3 CHECK (quality BETWEEN 1 AND 5),
    schema        INTEGER NOT NULL DEFAULT 1,
    content_hash  TEXT    NOT NULL,               -- sha256(body.trim_end())
    body          TEXT    NOT NULL,
    created_at    TEXT    NOT NULL,               -- RFC3339
    updated_at    TEXT    NOT NULL,
    deleted_at    TEXT,                           -- soft delete
    md_path       TEXT    NOT NULL                -- relative to data_dir
);
CREATE INDEX idx_memories_repo    ON memories(repo)
    WHERE deleted_at IS NULL;
CREATE INDEX idx_memories_kind    ON memories(kind)
    WHERE deleted_at IS NULL;
CREATE INDEX idx_memories_updated ON memories(updated_at);

CREATE TABLE memory_tags (
    memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    tag       TEXT NOT NULL,
    PRIMARY KEY (memory_id, tag)
);
CREATE INDEX idx_memory_tags_tag ON memory_tags(tag);

-- ─── memory vectors (sqlite-vec) ─────────────────────────────────────────
-- dim configurable via COMEMORY_VECTOR_DIM (default 1024).
-- the caller MUST send vectors of the configured dim; mismatch is
-- a hard error.
CREATE VIRTUAL TABLE memory_vec USING vec0(
    memory_id TEXT PRIMARY KEY,
    embedding FLOAT[1024]
);

-- ─── memory full-text (FTS5, contentless mirror) ─────────────────────────
CREATE VIRTUAL TABLE memory_fts USING fts5(
    memory_id UNINDEXED,
    body,
    tags,
    tokenize = 'porter unicode61 remove_diacritics 2'
);

-- ─── code symbols (BYO-vector or lexical-only) ───────────────────────────
CREATE TABLE code_symbols (
    id          INTEGER PRIMARY KEY,
    repo        TEXT    NOT NULL,
    path        TEXT    NOT NULL,                 -- relative to repo root
    blob_oid    TEXT    NOT NULL,                 -- git blob hash (incremental)
    symbol      TEXT    NOT NULL,                 -- qualified name
    kind        TEXT    NOT NULL,                 -- function/struct/...
    lang        TEXT    NOT NULL,                 -- rust/typescript/...
    line_start  INTEGER NOT NULL,
    line_end    INTEGER NOT NULL,
    snippet     TEXT    NOT NULL,                 -- raw text for FTS + display
    simhash     INTEGER NOT NULL,                 -- 64-bit SimHash of tokens
    indexed_at  TEXT    NOT NULL,
    UNIQUE (repo, path, symbol, line_start)
);
CREATE INDEX idx_code_repo_path ON code_symbols(repo, path);
CREATE INDEX idx_code_blob      ON code_symbols(blob_oid);
CREATE INDEX idx_code_simhash   ON code_symbols(simhash);

CREATE VIRTUAL TABLE code_vec USING vec0(
    symbol_id INTEGER PRIMARY KEY,
    embedding FLOAT[768]
);

CREATE VIRTUAL TABLE code_fts USING fts5(
    symbol_id UNINDEXED,
    symbol,
    snippet,
    path_tokens,                                  -- path split on /._-
    tokenize = 'unicode61 remove_diacritics 2'
);

-- ─── per-file indexing marker for incremental re-index ───────────────────
CREATE TABLE indexed_files (
    repo       TEXT NOT NULL,
    path       TEXT NOT NULL,
    blob_oid   TEXT NOT NULL,
    indexed_at TEXT NOT NULL,
    PRIMARY KEY (repo, path)
);

-- ─── graph edges (replaces kuzu) ─────────────────────────────────────────
-- node addressing:
--   memory:<id>
--   file:<repo>:<path>
--   symbol:<symbol_id>
--   repo:<repo>
--   author:<name>
--   tag:<name>
CREATE TABLE edges (
    src_kind   TEXT NOT NULL,
    src_id     TEXT NOT NULL,
    dst_kind   TEXT NOT NULL,
    dst_id     TEXT NOT NULL,
    rel        TEXT NOT NULL CHECK (rel IN
               ('in_repo','authored_by','tagged',
                'references_file','references_symbol',
                'relates_to','supersedes','conflicts_with','derived_from')),
    created_at TEXT NOT NULL,
    PRIMARY KEY (src_kind, src_id, dst_kind, dst_id, rel)
);
CREATE INDEX idx_edges_src ON edges(src_kind, src_id, rel);
CREATE INDEX idx_edges_dst ON edges(dst_kind, dst_id, rel);

-- ─── stats / feedback (kept from v0.1) ───────────────────────────────────
CREATE TABLE search_stats (
    id          INTEGER PRIMARY KEY,
    query       TEXT NOT NULL,
    hit_count   INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL,
    ran_at      TEXT NOT NULL
);

CREATE TABLE feedback (
    memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    delta     INTEGER NOT NULL,
    given_at  TEXT NOT NULL
);

-- ─── schema metadata ─────────────────────────────────────────────────────
CREATE TABLE schema_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
INSERT INTO schema_meta(key, value) VALUES
    ('version', '2'),
    ('memory_vector_dim', '1024'),
    ('code_vector_dim',   '768');
```

### 4.2 Query examples

```sql
-- BYO-vector memory search
SELECT m.id, m.slug, m.kind, v.distance
  FROM memory_vec v
  JOIN memories  m ON m.id = v.memory_id
 WHERE v.embedding MATCH :query_vec
   AND k = :k
   AND m.deleted_at IS NULL
   AND (:repo IS NULL OR m.repo = :repo)
 ORDER BY v.distance;

-- Lexical fallback (no vector provided)
SELECT m.id, m.slug, bm25(memory_fts) AS score
  FROM memory_fts
  JOIN memories m ON m.id = memory_fts.memory_id
 WHERE memory_fts MATCH :query
   AND m.deleted_at IS NULL
 ORDER BY score
 LIMIT :k;

-- Graph walk: transitive supersedes, depth ≤ 5
WITH RECURSIVE walk(id, depth) AS (
    SELECT :start_memory_id, 0
    UNION ALL
    SELECT e.src_id, w.depth + 1
      FROM edges e
      JOIN walk w ON e.dst_id = w.id
     WHERE e.src_kind = 'memory' AND e.dst_kind = 'memory'
       AND e.rel = 'supersedes'
       AND w.depth < 5
)
SELECT id FROM walk WHERE depth > 0;

-- Code similarity by SimHash bucket (Hamming ≤ 8 over 64 bits)
SELECT id, repo, path, symbol,
       bit_count(simhash ^ :query_simhash) AS hamming
  FROM code_symbols
 WHERE bit_count(simhash ^ :query_simhash) <= 8
 ORDER BY hamming
 LIMIT :k;
```

`bit_count()` requires SQLite ≥ 3.45. The rusqlite 0.32 bundled SQLite is
3.46, so this is satisfied. If a future downgrade ever lowers that
version, register a Rust user-defined function as a fallback.

### 4.3 Invariants

1. Markdown is the source of truth. `comemory rebuild` can always
   regenerate the entire SQLite database from `memories/*.md` plus the
   set of indexed repo roots.
2. `content_hash` proves a `memories` row matches its markdown body.
3. The vector dim for each table is set once via `schema_meta` and
   locked. The first successful `save --vector` (or `ingest-code`)
   commits the dim; later mismatches return `EX_DATAERR 65`.
4. Code vectors are optional. A symbol can live in `code_symbols` /
   `code_fts` without a row in `code_vec`.
5. The CLI never panics. All failures flow through the `Error` enum and
   map to sysexits codes in `cli::run`.
6. `PRAGMA busy_timeout = 5000` and WAL mode keep concurrent CLI
   invocations safe.

## 5. CLI Surface and Flows

### 5.1 Subcommand inventory

| Command | Status in v0.2 | Notes |
|---------|----------------|-------|
| `save` | changed | gains `--vector <FLOAT,FLOAT,...>` and `--vector-stdin` |
| `list` | unchanged | rows from `memories` |
| `show <ID>` | unchanged | reads markdown |
| `edit <ID>` | unchanged | `$EDITOR` round-trip + rehash |
| `delete <ID>` | unchanged | soft delete + edge cascade |
| `search <QUERY>` | changed | `--vector*` optional → ANN; else FTS5 BM25 |
| `context <QUERY>` | changed | `search` plus graph + code expansion |
| `index-code` | changed | lexical + SimHash only by default; `--extract` emits JSONL |
| `ingest-code` | **new** | reads pre-embedded JSONL into `code_symbols` + `code_vec` |
| `ast <PATTERN>` | unchanged | ast-grep pass-through (rust/ts/js/py/go) |
| `prune` | changed | SQL queries instead of kuzu walks |
| `rebuild` | **new** | drop + repopulate `comemory.db` from md + indexed repos |
| `doctor` | unchanged | DB writable, schema version, sqlite-vec loaded |
| `serve` | **removed** | drops axum + web stack |
| `stats`, `feedback` | unchanged | rusqlite tables |

Global flags `--data-dir <PATH>` and `--json` keep their v0.1 semantics
and can appear before or after the subcommand.

### 5.2 Save flow

```
$ comemory save \
    --kind decision \
    --tags db,postgres \
    --repo qwick-backend \
    --vector-stdin \
    "Use Postgres advisory locks for migration ordering." < vec.json

# vec.json
# {"embedding":[0.013,-0.221,...]}
```

1. Parse args, resolve `repo`/`author` from git or env.
2. Build `Frontmatter { schema: 1, content_hash: sha256(body.trim_end()), ... }`.
3. Atomic write: stage `memories/.<id>.tmp`, then `fs::rename` to
   `memories/<id>-<slug>.md`. On any failure between stage and rename,
   remove the tmp file.
4. Open SQLite (WAL, `busy_timeout = 5000 ms`, load sqlite-vec).
5. Transaction:
   - `INSERT INTO memories`.
   - `INSERT INTO memory_tags`.
   - `INSERT INTO memory_fts` (`body`, joined `tags`).
   - If `--vector` or `--vector-stdin`: validate dim against
     `schema_meta.memory_vector_dim`; on first vector ever, set the dim
     and lock it; `INSERT INTO memory_vec`.
   - `cross_link::extract_refs(body)` → `INSERT INTO edges`
     (`references_file`, `references_symbol`).
   - `INSERT` edges for `in_repo`, `authored_by`, `tagged`.
6. Commit. Markdown is canonical; on commit failure the markdown is
   already on disk and `rebuild` can recover.
7. If `COMEMORY_GIT_AUTO_SYNC=1`, run `git_utils::auto_sync` against the
   memories directory.

Exit codes:

- `0` success
- `EX_USAGE 64` bad flags
- `EX_DATAERR 65` vector dim mismatch, malformed YAML
- `EX_IOERR 74` markdown write failure
- `EX_SOFTWARE 70` SQLite failure (markdown preserved)

### 5.3 Search flow

```
$ comemory search "advisory lock migration" --vector-stdin --k 12 < vec.json
$ comemory search "advisory lock migration" --k 12
$ comemory search "advisory lock migration" --tag postgres --repo qwick-backend
```

```
                ┌─ --vector(-stdin) given? ─┐
                │                           │
              yes                          no
                │                           │
                ▼                           ▼
        sqlite-vec ANN over           FTS5 BM25 over
        memory_vec (KNN k)            memory_fts (LIMIT k)
                │                           │
                └──────────┬────────────────┘
                           ▼
                 frontmatter filters
                 (--repo --kind --tag --since)
                           ▼
                 quality blend (config weights)
                           ▼
                 emit rows (TTY or --json)
```

If both a vector and a query string are provided, run both branches and
fuse via Reciprocal Rank Fusion in `retrieval::rank`.

### 5.4 Context flow

`context <QUERY>` is `search` plus graph expansion, designed for piping
into an agent prompt.

1. Run search (vector, lexical, or hybrid) → top-K memory hits.
2. For each hit, walk edges depth ≤ 2 across `references_file`,
   `references_symbol`, `relates_to`, `supersedes` using a recursive
   CTE.
3. Resolve `symbol_id` / `file:repo:path` neighbors to `code_symbols`
   rows for snippet context.
4. Bundle JSON:

```json
{
  "query": "advisory lock",
  "memories":  [{ "id": "...", "kind": "...", "body": "...", "score": 0.91 }],
  "code_refs": [{ "repo": "...", "path": "...", "symbol": "...", "snippet": "..." }],
  "relations": [{ "from": "memory:abcd", "rel": "supersedes", "to": "memory:efgh" }]
}
```

### 5.5 Index-code flow

Default = lexical + SimHash only, no vectors:

```
$ comemory index-code --repo qwick-backend --path ./qwick-backend
    walked 4123 files
    skipped 3987 (blob_oid unchanged)
    indexed 1142 symbols across 136 files
```

1. Open the repo via `git2`. Enumerate paths via
   `ignore::Walk` honoring `.gitignore`.
2. For each file, compute `git2::Blob::id()`. Compare against
   `indexed_files.blob_oid`. Skip if unchanged.
3. Map file extension → language. Skip if not in the compiled set
   (rust, typescript, javascript, python, go).
4. Parse with `ast-grep-core`. Enumerate symbol nodes (function,
   struct, enum, method, trait, interface, class).
5. For each symbol:
   - Tokenize the snippet (alphanumeric runs, lowercased).
   - Compute `simhash = SimHash64(tokens)` using
     `siphasher`-based hashing.
   - `INSERT OR REPLACE INTO code_symbols`.
   - `INSERT INTO code_fts`.
   - Edges: `file:<repo>:<path>` ⇄ `symbol:<id>`.
6. Update `indexed_files` with the new `blob_oid` + `indexed_at`.

BYO-vector path (composable):

```
# Phase A: extract to JSONL, no DB writes
$ comemory index-code --repo qwick-backend --path . --extract \
    > symbols.jsonl

# Phase B: caller embeds each line, attaches an "embedding" field
$ ./embed.sh < symbols.jsonl > symbols-embedded.jsonl

# Phase C: ingest back
$ comemory ingest-code < symbols-embedded.jsonl
```

JSONL schema (one symbol per line):

```json
{"repo":"qwick-backend","path":"src/db/migrate.rs","blob_oid":"a1b2…",
 "symbol":"apply_migrations","kind":"function","lang":"rust",
 "line_start":42,"line_end":91,"snippet":"fn apply_migrations(...) { ... }",
 "simhash":4523109887554234899,"embedding":[0.012,-0.331,...]}
```

`ingest-code` validates the dim against `schema_meta.code_vector_dim`.

### 5.6 Ast flow

Same contract as v0.1, but `--lang` is restricted to the compiled
language set: `rust`, `typescript`, `javascript`, `python`, `go`.
Passing any other value returns `EX_USAGE 64` with a message listing
the supported languages.

```
$ comemory ast 'fn $NAME($$$) -> Result<$RET> { $$$ }' --lang rust src/
```

Pass-through to `ast-grep-core`. Returns matches as TTY or `--json`.

### 5.7 Wrapper helper

`scripts/comemory-embed.sh` ships in the repo as the "just-works" UX
without bundling a model:

```bash
#!/usr/bin/env bash
# Bridges comemory ↔ Ollama for the BYO-vector flow.
# Usage:
#   comemory-embed save --kind decision "body text"
#   comemory-embed search "query"
set -euo pipefail
: "${COMEMORY_EMBED_URL:=http://localhost:11434/api/embeddings}"
: "${COMEMORY_EMBED_MODEL:=nomic-embed-text}"

embed() {
    local text="$1"
    curl -fsS "$COMEMORY_EMBED_URL" \
        -d "$(jq -n --arg m "$COMEMORY_EMBED_MODEL" --arg t "$text" \
              '{model:$m, prompt:$t}')" \
      | jq -c '{embedding}'
}

cmd="$1"; shift
case "$cmd" in
    save)
        body="${@: -1}"
        embed "$body" | comemory save --vector-stdin "$@" ;;
    search)
        query="$1"; shift
        embed "$query" | comemory search "$query" --vector-stdin "$@" ;;
    *) echo "usage: comemory-embed save|search ..."; exit 64 ;;
esac
```

The wrapper is documentation, not enforcement. Callers are free to
embed however they want.

## 6. Module Deltas

### 6.1 `src/` deltas

| Path | Action | Reason |
|------|--------|--------|
| `src/cli/serve*` | delete | drop `serve` subcommand |
| `src/cli/{save,search,context,index_code,ast,prune,doctor,list,show,edit,delete}.rs` | rewrite | rewire to new storage |
| `src/cli/rebuild.rs` | new | drop + repopulate `comemory.db` |
| `src/cli/ingest_code.rs` | new | reads JSONL of pre-embedded symbols |
| `src/index/embedder.rs` | delete | fastembed gone |
| `src/index/{memory_index,code_index,schema}.rs` | delete | lancedb gone |
| `src/index/mod.rs` | rewrite | thin wrapper over `src/store/` |
| `src/graph/` (kuzu schema + queries) | delete | kuzu gone |
| `src/graph/mod.rs` (replaced) | new | edges/walk helpers on rusqlite |
| `src/graph/cross_link.rs` | keep, rewire | still parses backticked refs; emits SQL inserts |
| `src/retrieval/{router,hybrid,corrective,rank,bundle}.rs` | rewrite | run over SQLite, drop datafusion plans |
| `src/ast/{extractor,pattern,languages}.rs` | trim | only rust/ts/js/py/go wired |
| `src/stats/` | keep | rusqlite already |
| `src/store/` | new module | central SQLite layer: connection pool, schema_meta, migrations, sqlite-vec loader |
| `src/store/{connection,schema,migrate,vector,fts}.rs` | new | one file per concern (≤500 LoC) |
| `src/store/embed.rs` | new | `to_vec_blob(&[f32]) -> Vec<u8>`, `dim_guard`, vector helpers |
| `src/simhash.rs` | new | 64-bit SimHash over token iterator, siphasher-based |
| `src/output/` | keep | TTY + JSON emitters unchanged |
| `src/config/` | keep, extend | new env vars (§7) |
| `src/prune/` | rewrite | SQL queries instead of kuzu |
| `src/memory/` | keep | markdown I/O + frontmatter unchanged |
| `src/git_utils.rs` | keep, extend | add `blob_oid_for(path)` helper |
| `src/errors.rs` | trim | drop `Kuzu`, `Lance`, `Fastembed`; add `Sqlite`, `VecDimMismatch`, `Migration` |
| `src/prelude.rs` | trim | drop kuzu/lance re-exports |
| `src/lib.rs`, `src/main.rs` | minor | wire new modules, drop `serve` |

### 6.2 `Cargo.toml` deltas

```toml
# REMOVED
arrow-array
arrow-schema
axum
fastembed
kuzu
lancedb
mime_guess
open
rust-embed
tower
tower-http

# CHANGED
rusqlite = { version = "0.32",
             features = ["bundled", "load_extension", "vtab", "blob"] }

ast-grep-language = { version = "0.38",
                      default-features = false,
                      features = ["tree-sitter-rust",
                                  "tree-sitter-typescript",
                                  "tree-sitter-javascript",
                                  "tree-sitter-python",
                                  "tree-sitter-go"] }

# NEW
sqlite-vec = { version = "0.1", default-features = false }

[build-dependencies]
cc = "1"

# PROFILE — eh_frame + gcc_except_tab savings
[profile.release]
lto = "fat"
codegen-units = 1
strip = "symbols"
panic = "abort"          # NEW
```

`cargo deny check` must pass after the swap. sqlite-vec is licensed
Apache-2.0/MIT.

### 6.3 `build.rs`

A new `build.rs` compiles `sqlite-vec` C source into a static library
and exposes its entry point so `Connection::load_extension` (against
the in-process bundled SQLite) succeeds at runtime without any
filesystem dependency. The build script also guards platform support
for the four cargo-dist targets:

- `aarch64-apple-darwin`
- `x86_64-apple-darwin`
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`

Failures fast at build time, not at runtime.

### 6.4 Error handling

```rust
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("io: {0}")] Io(#[from] std::io::Error),
    #[error("sqlite: {0}")] Sqlite(#[from] rusqlite::Error),
    #[error("yaml: {0}")] Yaml(#[from] serde_yaml::Error),
    #[error("json: {0}")] Json(#[from] serde_json::Error),
    #[error("ast: {0}")] Ast(String),
    #[error("git: {0}")] Git(#[from] git2::Error),

    #[error("vector dim mismatch: expected {expected}, got {got}")]
    VecDimMismatch { expected: usize, got: usize },

    #[error("schema migration failed: {0}")] Migration(String),
    #[error("invalid frontmatter: {0}")]     Frontmatter(String),
    #[error("memory not found: {0}")]        NotFound(String),
    #[error("config: {0}")]                  Config(String),
}

pub type Result<T> = std::result::Result<T, Error>;
```

`cli::run` maps:

| Variant | Sysexits | Code |
|---------|----------|------|
| `Sqlite`, `Migration` | `EX_SOFTWARE` | 70 |
| `Io` | `EX_IOERR` | 74 |
| `Yaml`, `Json`, `Frontmatter`, `VecDimMismatch` | `EX_DATAERR` | 65 |
| `NotFound` | `EX_USAGE` | 64 |
| `Config` | `EX_CONFIG` | 78 |
| `Ast`, `Git` | `EX_SOFTWARE` | 70 |

### 6.5 Gate compliance

All five binding rules from `CLAUDE.md` continue to apply unchanged:

1. No duplication (`scripts/dup-check.sh`).
2. Modular files.
3. ≤500 lines per file in `src/` and `scripts/`
   (`scripts/module-size-check.sh`).
4. Zero errors, zero warnings (`scripts/no-bypass-check.sh`).
5. Tests in `tests/` mirror `src/` 1:1
   (`scripts/test-placement-check.sh`,
   `scripts/tests-mirror-check.sh`).

## 7. Configuration

| Variable | Purpose | Default |
|----------|---------|---------|
| `COMEMORY_DATA_DIR` | Root data directory | `~/.comemory` |
| `COMEMORY_VECTOR_DIM` | Memory vector dimension (locked on first save) | `1024` |
| `COMEMORY_CODE_VECTOR_DIM` | Code vector dimension (locked on first ingest) | `768` |
| `COMEMORY_INDEXING_AUTO_REINDEX` | `lazy` \| `hook` \| `off` | `lazy` |
| `COMEMORY_RETRIEVAL_TOP_K` | Top-k for hybrid router | `12` |
| `COMEMORY_RETRIEVAL_MEMORY_THRESHOLD` | Min cosine for memory hits | `0.55` |
| `COMEMORY_RETRIEVAL_CODE_THRESHOLD` | Min cosine for code hits | `0.50` |
| `COMEMORY_GIT_AUTO_SYNC` | `1` to enable best-effort `git commit && git push` after save | `false` |
| `COMEMORY_EMBED_HINT` | Free-form caller-set hint (e.g. `ollama:nomic-embed-text`) shown by `doctor`; informational only | _(unset)_ |

CLI `--data-dir` and `--json` remain global flags.

## 8. Testing

- Runner: `cargo nextest run --all-features` (alias `just test`). Never
  plain `cargo test`.
- `tests/` mirrors `src/` 1:1. Each top-level test binary
  (`tests/<module>.rs`) is a thin shim that declares submodules under
  `tests/<module>/`.
- `tests/common/` holds shared fixtures (temp data-dir builders, sample
  markdown memories, deterministic vector generators).

Concrete additions / changes:

- **new** `tests/store/{connection,schema,migrate,vector,fts}.rs`.
- **new** `tests/store/embed.rs` — dim guard + blob round-trip.
- **new** `tests/simhash.rs` — collision distribution, Hamming sanity.
- **new** `tests/graph/edges.rs` — recursive CTE for `supersedes` and
  `references_*`.
- **changed** `tests/cli/save.rs` — covers vector / no-vector branches,
  dim-lock enforcement.
- **changed** `tests/cli/search.rs` — lexical, vector, hybrid blend.
- **changed** `tests/cli/index_code.rs` — `blob_oid` incremental,
  language gating, `--extract` JSONL output.
- **new** `tests/cli/ingest_code.rs` — JSONL round-trip, dim mismatch.
- **new** `tests/cli/rebuild.rs` — markdown → DB reconstruction
  equivalence.
- **changed** `tests/retrieval/router.rs` — vector-present vs absent
  branching, fusion correctness.
- **refreshed** `tests/snapshots/*.snap` — insta snapshots regenerated.

Deleted:

- `tests/graph/kuzu*`, `tests/index/lance*`, `tests/index/embedder*`,
  `tests/serve/*`.

Test policy reminders:

- Real data only. No mocks. Memory fixtures are real markdown files;
  vector fixtures are deterministic float vectors stored in
  `tests/common/vectors.rs` (generated once, checked in). Tests assert
  ranking order, not absolute floats.
- For `index-code`, a small fixture repo under
  `tests/common/fixtures/sample-repo/` carries a real `.git/` directory
  generated by `tests/common/git_setup.rs`.
- `.config/nextest.toml` removes the `embedder|memory_index|code_index`
  serialization group; no model downloads happen in tests.

## 9. Migration

Project is pre-1.0. v0.2 ignores any pre-existing `.comemory/lancedb/`
or `.comemory/kuzu/` directories — they can be deleted manually after a
successful `comemory rebuild`. `memories/*.md` is the only artifact
that survives.

Release notes / `CHANGELOG.md` will call out the break explicitly and
direct users to:

1. Upgrade comemory.
2. Run `comemory rebuild` to populate the new `comemory.db`.
3. Re-embed memories via the BYO-vector wrapper if semantic search is
   desired.
4. Re-run `comemory index-code` for indexed repos.

## 10. Risks

1. **`sqlite-vec` maturity.** Pre-1.0 upstream. The text-primary-key
   syntax used in the `memory_vec` virtual table
   (`memory_id TEXT PRIMARY KEY`) requires `sqlite-vec ≥ 0.1.6`. If the
   pinned version does not support it, the implementation falls back to
   an integer rowid plus a `memory_vec_map(rowid INTEGER, memory_id TEXT
   UNIQUE)` mapping table. Mitigation: pin to a known-good version in
   `Cargo.toml`; cover both schemas behind the `store::schema` module so
   the switch is a one-file change.
2. **`bit_count()` availability.** Requires SQLite ≥ 3.45. rusqlite
   0.32 bundles 3.46. Mitigation: assert version at boot; on older
   builds register a Rust UDF.
3. **`ast-grep-language` no-default-features.** Need to confirm the
   `tree-sitter-*` sub-features don't transitively re-enable extra
   parsers. Mitigation: re-run `cargo bloat --crates` after the swap
   and confirm the size delta.
4. **Sweep-the-board test churn.** `tests-mirror-check.sh` and
   `test-placement-check.sh` will fail during the refactor until the
   `tests/` tree is updated in lockstep. Mitigation: land everything in
   one PR; `scripts/check-all.sh` is the merge gate.
5. **Cross-platform builds.** All four cargo-dist targets must compile
   the vendored sqlite-vec C source cleanly. Mitigation: validate in a
   cargo-dist dry-run plan before tagging the first v0.2 release.
6. **Wrapper-script UX gap.** Without an in-process embedder, the
   first-run experience for a new user is "set up Ollama". Mitigation:
   document the wrapper in README; lexical fallback always works, so
   the tool is usable without any embedder configured.

## 11. Out of Scope

- A C dynamic-load fallback for sqlite-vec.
- Restoring `comemory serve` or any web UI.
- Multi-machine sync of `comemory.db`.
- A per-repo JSON sidecar (revisit later if portability becomes a real
  need).
- Bundling an embedding model.
- Backwards compatibility with v0.1 data directories.

---
