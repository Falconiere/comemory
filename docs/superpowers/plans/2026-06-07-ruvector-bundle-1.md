# RuVector Bundle 1: Embed-on-Save + BM25 Hybrid + Benches

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the v1.1 "memory not embedded on save" gap, add lexical BM25 retrieval fused with the dense LanceDB index via Reciprocal Rank Fusion, and publish reproducible retrieval/save benchmarks.

**Architecture:**
1. `cli/save.rs` opens a shared `MemoryIndex` + `Embedder` and upserts after the markdown rename succeeds. Failures warn-and-swallow like the existing graph upsert.
2. A new SQLite FTS5 virtual table at `<data_dir>/index/fts.sqlite` mirrors memory bodies; `cli/save.rs` writes there too. `retrieval/fts.rs` queries it and returns BM25-ranked memory ids. `retrieval/fuse.rs` applies Reciprocal Rank Fusion over dense + sparse rankings and reifies `MemoryHit` rows from the dense table.
3. A new `benches/` workspace under criterion measures `Embedder::embed_one`, `MemoryIndex::upsert`, vector-only `search_memory`, and fused `search_memory_fused`. `scripts/bench.sh` + `just bench` produce a Markdown report under `docs/bench/`.

**Tech Stack:** Rust 1.95, `lancedb 0.29`, `fastembed 4` (nomic-embed-text-v1.5-Q), `rusqlite 0.32` (bundled FTS5), `criterion 0.5`, `tempfile`, `tokio`.

---

## File Structure

**Create:**
- `src/index/fts.rs` — SQLite FTS5 wrapper (`Fts::open`, `upsert`, `delete`, `search`). ≤200 LOC.
- `src/retrieval/fts.rs` — Async-friendly query wrapper around `Fts::search`. ≤80 LOC.
- `src/retrieval/fuse.rs` — Dense ⊕ sparse RRF fusion that reifies `MemoryHit` rows. ≤150 LOC.
- `tests/index/fts.rs` — FTS5 upsert/search/delete tests.
- `tests/retrieval/fts.rs` — Query wrapper tests.
- `tests/retrieval/fuse.rs` — Fusion correctness + tie-break tests.
- `benches/retrieval.rs` — Criterion benches for vector search and fused search.
- `benches/save.rs` — Criterion benches for embed + memory index upsert.
- `benches/common.rs` — Shared fixture (sandboxed data dir + seeded memories).
- `scripts/bench.sh` — Reproducible bench runner that pipes results to `docs/bench/latest.md`.
- `docs/bench/README.md` — How to read the numbers + reproducibility caveat.

**Modify:**
- `src/cli/save.rs` — Embed body and upsert into `MemoryIndex` + `Fts`; both best-effort.
- `src/cli/search.rs` — Replace `search_memory` call with `search_memory_fused`; route still observed.
- `src/index/mod.rs` — Re-export `Fts`.
- `src/retrieval/mod.rs` — Re-export `search_memory_fused` and `fts::search_fts_ids`.
- `src/retrieval/rank.rs` — Add `rrf_fuse(rankings, k_const)` pure helper.
- `src/retrieval/hybrid.rs` — Doc-only update: this module is now strictly vector-only; fusion lives in `retrieval::fuse`.
- `src/config/file.rs` — Add `RetrievalConfig.rrf_k` (default `60.0`) + env var `QWICK_MEMORY_RETRIEVAL_RRF_K`.
- `Cargo.toml` — Add `criterion 0.5` dev-dep with `[[bench]]` entries; ensure `rusqlite` retains `bundled` (FTS5 ships in `SQLITE_ENABLE_FTS5` bundled build).
- `justfile` — Add `bench` recipe wrapping `scripts/bench.sh`.
- `README.md` — Replace the "memory not embedded on save" line in v1.1 gaps with a "benchmark" subsection pointing at `docs/bench/`.

---

## Task 1: Add `rrf_fuse` helper to `retrieval/rank.rs`

Reciprocal Rank Fusion: for each list of ids, contribute `1 / (k + rank)` to the id's total score (1-indexed rank). Pure. No I/O.

**Files:**
- Modify: `src/retrieval/rank.rs`
- Test: `tests/retrieval/rank.rs`

- [ ] **Step 1: Write failing tests for `rrf_fuse`**

Append to `tests/retrieval/rank.rs`:

```rust
use comemory::retrieval::rank::rrf_fuse;

#[test]
fn rrf_fuse_empty_inputs_returns_empty() {
    let out: Vec<(String, f32)> = rrf_fuse::<&str>(&[], 60.0);
    assert!(out.is_empty());
}

#[test]
fn rrf_fuse_single_ranking_preserves_order() {
    let ranking = vec!["a", "b", "c"];
    let out = rrf_fuse::<&str>(&[&ranking], 60.0);
    let ids: Vec<&str> = out.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, vec!["a", "b", "c"]);
}

#[test]
fn rrf_fuse_two_rankings_boosts_intersection() {
    let r1 = vec!["a", "b", "c"];
    let r2 = vec!["c", "a", "d"];
    let out = rrf_fuse::<&str>(&[&r1, &r2], 60.0);
    let ids: Vec<&str> = out.iter().map(|(id, _)| id.as_str()).collect();
    // "a" (1+2) and "c" (3+1) both score higher than "b" or "d" (one list only).
    assert_eq!(ids[0], "a");
    assert_eq!(ids[1], "c");
    assert!(ids.contains(&"b"));
    assert!(ids.contains(&"d"));
}

#[test]
fn rrf_fuse_stable_tie_break_by_id() {
    let r1 = vec!["b", "a"];
    let r2 = vec!["a", "b"];
    let out = rrf_fuse::<&str>(&[&r1, &r2], 60.0);
    // Equal scores: id-ascending wins so output is reproducible.
    let ids: Vec<&str> = out.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, vec!["a", "b"]);
}
```

- [ ] **Step 2: Run test, expect compile failure**

Run: `cargo nextest run --all-features -E 'test(rrf_fuse)'`
Expected: FAIL — `unresolved import comemory::retrieval::rank::rrf_fuse`.

- [ ] **Step 3: Implement `rrf_fuse`**

Append to `src/retrieval/rank.rs`:

```rust
use std::collections::HashMap;

/// Reciprocal Rank Fusion. Each input is a ranking (best first); the score for
/// an id is the sum of `1 / (k + rank)` (1-indexed) across every ranking it
/// appears in. Output is sorted by score descending, with ascending id as a
/// stable tie-break so callers get deterministic ordering.
///
/// `k` is the RRF constant (typical value `60.0`); larger values flatten the
/// curve so deeper-rank hits matter more relative to top-of-list hits.
pub fn rrf_fuse<S>(rankings: &[&[S]], k: f32) -> Vec<(String, f32)>
where
    S: AsRef<str>,
{
    let mut scores: HashMap<String, f32> = HashMap::new();
    for ranking in rankings {
        for (i, id) in ranking.iter().enumerate() {
            let rank = (i + 1) as f32;
            *scores.entry(id.as_ref().to_string()).or_insert(0.0) += 1.0 / (k + rank);
        }
    }
    let mut out: Vec<(String, f32)> = scores.into_iter().collect();
    out.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    out
}
```

- [ ] **Step 4: Run tests, expect pass**

Run: `cargo nextest run --all-features -E 'test(rrf_fuse)'`
Expected: PASS — 4 tests.

- [ ] **Step 5: Commit**

```bash
git add src/retrieval/rank.rs tests/retrieval/rank.rs
git commit -m "feat(retrieval): add reciprocal rank fusion helper

RRF combines dense (vector) and sparse (BM25) rankings deterministically.
Pure, stateless, sorted with stable id-tie-break so search output is
reproducible. Wired into hybrid retrieval in a follow-up task."
```

---

## Task 2: Scaffold `index::Fts` (FTS5 BM25 store)

Open a SQLite database at `<data_dir>/index/fts.sqlite`. Create a `memory_fts` virtual table on first open. No reads/writes yet.

**Files:**
- Create: `src/index/fts.rs`
- Modify: `src/index/mod.rs`
- Test: `tests/index/fts.rs`

- [ ] **Step 1: Write failing scaffold test**

Create `tests/index/fts.rs`:

```rust
use comemory::config::paths::Paths;
use comemory::index::Fts;

use super::common;

#[test]
fn open_creates_db_and_table() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let _fts = Fts::open(paths.vectors_dir().join("fts.sqlite")).unwrap();
    assert!(paths.vectors_dir().join("fts.sqlite").exists());
}
```

Append a module line to `tests/index.rs` so the new file is picked up:

```rust
mod fts;
```

- [ ] **Step 2: Run test, expect compile failure**

Run: `cargo nextest run --all-features -E 'test(open_creates_db_and_table)'`
Expected: FAIL — `unresolved import comemory::index::Fts`.

- [ ] **Step 3: Implement `Fts::open`**

Create `src/index/fts.rs`:

```rust
//! SQLite FTS5-backed lexical index over memory bodies. Mirrors what
//! `MemoryIndex` does for dense vectors: open/upsert/search/delete. The
//! `memory_fts` virtual table uses the default `unicode61` tokenizer with
//! `remove_diacritics=2`; the `id` column is `UNINDEXED` so FTS treats it
//! purely as a payload row key.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::prelude::*;

/// Connection to the FTS5-backed memory body index. Cheap to open per call —
/// SQLite holds a small file handle and the virtual table is built once.
pub struct Fts {
    conn: Connection,
}

impl Fts {
    /// Open (or create) the FTS5 database at `path`. The parent directory
    /// must already exist; `Paths::ensure_dirs` guarantees that for the
    /// default data layout.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(|e| Error::Other(e.to_string()))?;
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts \
             USING fts5(id UNINDEXED, body, tokenize = 'unicode61 remove_diacritics 2');",
        )
        .map_err(|e| Error::Other(e.to_string()))?;
        Ok(Self { conn })
    }
}
```

Modify `src/index/mod.rs`: add `pub mod fts;` and `pub use fts::Fts;` next to the existing exports.

- [ ] **Step 4: Run test, expect pass**

Run: `cargo nextest run --all-features -E 'test(open_creates_db_and_table)'`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/index/fts.rs src/index/mod.rs tests/index/fts.rs tests/index.rs
git commit -m "feat(index): scaffold SQLite FTS5 wrapper

Opens a memory_fts virtual table with unicode61 tokenizer. Upsert and
search land in the next two tasks; this commit only proves the schema
creates cleanly under the existing Paths layout."
```

---

## Task 3: `Fts::upsert` + `Fts::delete`

**Files:**
- Modify: `src/index/fts.rs`
- Test: `tests/index/fts.rs`

- [ ] **Step 1: Write failing tests**

Append to `tests/index/fts.rs`:

```rust
#[test]
fn upsert_then_count_returns_one() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let fts = Fts::open(paths.vectors_dir().join("fts.sqlite")).unwrap();
    fts.upsert("a1b2c3d4", "Use Postgres for analytics").unwrap();
    assert_eq!(fts.count().unwrap(), 1);
}

#[test]
fn upsert_same_id_overwrites() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let fts = Fts::open(paths.vectors_dir().join("fts.sqlite")).unwrap();
    fts.upsert("a1b2c3d4", "first body").unwrap();
    fts.upsert("a1b2c3d4", "second body").unwrap();
    assert_eq!(fts.count().unwrap(), 1);
}

#[test]
fn delete_removes_row() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let fts = Fts::open(paths.vectors_dir().join("fts.sqlite")).unwrap();
    fts.upsert("a1b2c3d4", "body").unwrap();
    fts.delete("a1b2c3d4").unwrap();
    assert_eq!(fts.count().unwrap(), 0);
}
```

- [ ] **Step 2: Run tests, expect fail**

Run: `cargo nextest run --all-features -E 'test(upsert) | test(delete_removes_row)' -E 'test(/upsert_/) | test(delete_removes_row)'`
Expected: FAIL — methods not found.

- [ ] **Step 3: Implement `upsert`, `delete`, `count`**

Append to `src/index/fts.rs`:

```rust
impl Fts {
    /// Insert or overwrite the body indexed under `id`. Implemented as
    /// `DELETE`+`INSERT` inside a single transaction because FTS5 virtual
    /// tables do not support `ON CONFLICT` upserts. The transaction keeps
    /// the row count correct under concurrent saves of the same id.
    pub fn upsert(&self, id: &str, body: &str) -> Result<()> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| Error::Other(e.to_string()))?;
        tx.execute("DELETE FROM memory_fts WHERE id = ?1;", [id])
            .map_err(|e| Error::Other(e.to_string()))?;
        tx.execute(
            "INSERT INTO memory_fts (id, body) VALUES (?1, ?2);",
            [id, body],
        )
        .map_err(|e| Error::Other(e.to_string()))?;
        tx.commit().map_err(|e| Error::Other(e.to_string()))?;
        Ok(())
    }

    /// Remove the row indexed under `id`. No-op when the id is not present.
    pub fn delete(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM memory_fts WHERE id = ?1;", [id])
            .map_err(|e| Error::Other(e.to_string()))?;
        Ok(())
    }

    /// Number of indexed rows. Used by tests and `comemory doctor`.
    pub fn count(&self) -> Result<usize> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM memory_fts;", [], |row| row.get(0))
            .map_err(|e| Error::Other(e.to_string()))?;
        Ok(n.max(0) as usize)
    }
}
```

- [ ] **Step 4: Run tests, expect pass**

Run: `cargo nextest run --all-features -E 'test(/upsert_/) | test(delete_removes_row)'`
Expected: PASS — 3 tests.

- [ ] **Step 5: Commit**

```bash
git add src/index/fts.rs tests/index/fts.rs
git commit -m "feat(index): FTS upsert/delete/count

Upsert is delete+insert in one transaction because FTS5 lacks ON CONFLICT.
Count is exposed for doctor and bench fixtures."
```

---

## Task 4: `Fts::search` (BM25-ranked id list)

**Files:**
- Modify: `src/index/fts.rs`
- Test: `tests/index/fts.rs`

- [ ] **Step 1: Write failing tests**

Append to `tests/index/fts.rs`:

```rust
#[test]
fn search_returns_relevant_ids_in_score_order() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let fts = Fts::open(paths.vectors_dir().join("fts.sqlite")).unwrap();
    fts.upsert("id1", "postgres analytics decision").unwrap();
    fts.upsert("id2", "redis cache notes").unwrap();
    fts.upsert("id3", "postgres migration race").unwrap();

    let hits = fts.search("postgres", 10).unwrap();
    let ids: Vec<&str> = hits.iter().map(|h| h.id.as_str()).collect();
    assert!(ids.contains(&"id1"));
    assert!(ids.contains(&"id3"));
    assert!(!ids.contains(&"id2"));
}

#[test]
fn search_respects_limit() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let fts = Fts::open(paths.vectors_dir().join("fts.sqlite")).unwrap();
    for i in 0..5 {
        fts.upsert(&format!("id{i}"), "postgres").unwrap();
    }
    let hits = fts.search("postgres", 3).unwrap();
    assert_eq!(hits.len(), 3);
}

#[test]
fn search_empty_query_returns_empty() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let fts = Fts::open(paths.vectors_dir().join("fts.sqlite")).unwrap();
    fts.upsert("id1", "postgres").unwrap();
    let hits = fts.search("", 5).unwrap();
    assert!(hits.is_empty());
}
```

- [ ] **Step 2: Run tests, expect fail**

Run: `cargo nextest run --all-features -E 'test(search_returns_relevant_ids_in_score_order) | test(search_respects_limit) | test(search_empty_query_returns_empty)'`
Expected: FAIL — `search` not found.

- [ ] **Step 3: Implement `search` + `FtsHit`**

Append to `src/index/fts.rs`:

```rust
/// One BM25 hit. `score` is the negated `bm25()` value (FTS5 returns negative
/// scores where smaller means more relevant); we flip the sign so callers can
/// sort descending uniformly with `MemoryHit::score`.
#[derive(Debug, Clone)]
pub struct FtsHit {
    /// Memory id stored in the `UNINDEXED` payload column.
    pub id: String,
    /// `-bm25()` so higher is more relevant.
    pub score: f32,
}

impl Fts {
    /// BM25 search. Empty / whitespace-only queries short-circuit to an empty
    /// result so callers don't have to filter them. The raw query string is
    /// passed straight to FTS5 — callers that want phrase or column filters
    /// can pass standard FTS5 syntax.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<FtsHit>> {
        if query.trim().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, bm25(memory_fts) AS s FROM memory_fts \
                 WHERE memory_fts MATCH ?1 \
                 ORDER BY s ASC LIMIT ?2;",
            )
            .map_err(|e| Error::Other(e.to_string()))?;
        let rows = stmt
            .query_map(rusqlite::params![query, limit as i64], |row| {
                let id: String = row.get(0)?;
                let raw: f64 = row.get(1)?;
                Ok(FtsHit {
                    id,
                    score: -raw as f32,
                })
            })
            .map_err(|e| Error::Other(e.to_string()))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| Error::Other(e.to_string()))?);
        }
        Ok(out)
    }
}
```

- [ ] **Step 4: Run tests, expect pass**

Run: `cargo nextest run --all-features -E 'test(search_returns_relevant_ids_in_score_order) | test(search_respects_limit) | test(search_empty_query_returns_empty)'`
Expected: PASS — 3 tests.

- [ ] **Step 5: Commit**

```bash
git add src/index/fts.rs tests/index/fts.rs
git commit -m "feat(index): FTS BM25 search returning ranked ids

Negates FTS5's bm25() so higher score = more relevant, matching
MemoryHit::score conventions. Empty queries short-circuit so the
fused retrieval path can pass user input through unchanged."
```

---

## Task 5: `retrieval::fts` query wrapper

Thin module that owns the open-and-search choreography so callers don't reach into `index::Fts` directly.

**Files:**
- Create: `src/retrieval/fts.rs`
- Modify: `src/retrieval/mod.rs`
- Test: `tests/retrieval/fts.rs`

- [ ] **Step 1: Write failing test**

Create `tests/retrieval/fts.rs`:

```rust
use comemory::config::paths::Paths;
use comemory::index::Fts;
use comemory::retrieval::fts::search_fts_ids;

use super::common;

#[test]
fn search_fts_ids_returns_bm25_ordered_ids() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let db = paths.vectors_dir().join("fts.sqlite");
    let fts = Fts::open(&db).unwrap();
    fts.upsert("id_match", "postgres analytics").unwrap();
    fts.upsert("id_miss", "redis cache").unwrap();
    drop(fts);

    let ids = search_fts_ids(&db, "postgres", 5).unwrap();
    assert_eq!(ids.first().map(String::as_str), Some("id_match"));
    assert!(!ids.iter().any(|x| x == "id_miss"));
}

#[test]
fn search_fts_ids_returns_empty_when_db_missing() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let ids = search_fts_ids(&paths.vectors_dir().join("missing.sqlite"), "q", 5).unwrap();
    assert!(ids.is_empty());
}
```

Append a module line to `tests/retrieval.rs`:

```rust
mod fts;
```

- [ ] **Step 2: Run test, expect compile failure**

Run: `cargo nextest run --all-features -E 'test(search_fts_ids_returns_bm25_ordered_ids) | test(search_fts_ids_returns_empty_when_db_missing)'`
Expected: FAIL — `unresolved import comemory::retrieval::fts::search_fts_ids`.

- [ ] **Step 3: Implement `search_fts_ids`**

Create `src/retrieval/fts.rs`:

```rust
//! BM25 query wrapper around `index::Fts`. Exposes a single async-friendly
//! function the retrieval pipeline can call without taking on a SQLite
//! connection in its own state.

use std::path::Path;

use crate::index::Fts;
use crate::prelude::*;

/// Open the FTS5 database at `db_path` and return up to `limit` memory ids
/// ranked by BM25 relevance to `query`. When the database file does not yet
/// exist (no `comemory save` has run, or the file was deleted) we return an
/// empty list rather than erroring so the fused retrieval path can degrade to
/// vector-only without special-casing.
pub fn search_fts_ids(db_path: impl AsRef<Path>, query: &str, limit: usize) -> Result<Vec<String>> {
    if !db_path.as_ref().exists() {
        return Ok(Vec::new());
    }
    let fts = Fts::open(db_path)?;
    let hits = fts.search(query, limit)?;
    Ok(hits.into_iter().map(|h| h.id).collect())
}
```

Modify `src/retrieval/mod.rs`: add `pub mod fts;`.

- [ ] **Step 4: Run tests, expect pass**

Run: `cargo nextest run --all-features -E 'test(search_fts_ids_returns_bm25_ordered_ids) | test(search_fts_ids_returns_empty_when_db_missing)'`
Expected: PASS — 2 tests.

- [ ] **Step 5: Commit**

```bash
git add src/retrieval/fts.rs src/retrieval/mod.rs tests/retrieval/fts.rs tests/retrieval.rs
git commit -m "feat(retrieval): wrap FTS5 BM25 query as a pure function

Returns ranked ids or an empty list when the FTS db has not been
created yet (no save has run). Lets the fused search path stay
identical for cold and warm corpora."
```

---

## Task 6: `retrieval::fuse::search_memory_fused`

Calls vector search + FTS search, RRF-fuses the two ranked id lists, then materializes `MemoryHit` rows from the dense table (which already carries body/repo/kind).

**Files:**
- Create: `src/retrieval/fuse.rs`
- Modify: `src/retrieval/mod.rs`
- Modify: `src/config/file.rs` (add `rrf_k`)
- Test: `tests/retrieval/fuse.rs`

- [ ] **Step 1: Add `rrf_k` to `RetrievalConfig`**

Edit the `RetrievalConfig` struct in `src/config/file.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalConfig {
    pub memory_threshold: f32,
    pub code_threshold: f32,
    pub hybrid_weight: f32,
    pub top_k: usize,
    pub corrective_min_confidence: f32,
    /// RRF constant for sparse/dense fusion. Default 60.0 matches the original
    /// Cormack/Clarke/Buettcher RRF paper.
    pub rrf_k: f32,
}
```

In the `Config::defaults` body (same file), set `rrf_k: 60.0`. In `Config::with_env`, add a parse arm:

```rust
if let Ok(v) = std::env::var("QWICK_MEMORY_RETRIEVAL_RRF_K") {
    if let Ok(parsed) = v.parse::<f32>() {
        self.retrieval.rrf_k = parsed;
    }
}
```

(Match the existing env-parse style for `memory_threshold` already in that file.)

- [ ] **Step 2: Write failing test**

Create `tests/retrieval/fuse.rs`:

```rust
use comemory::config::paths::Paths;
use comemory::index::{Embedder, Fts, MemoryIndex};
use comemory::memory::{Kind, MemoryStore};
use comemory::retrieval::fuse::search_memory_fused;

use super::common;

#[tokio::test]
async fn fused_search_finds_lexical_only_match() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();

    let store = MemoryStore::new(paths.clone());
    let rec = store
        .save(
            "The arcane phrase zzzyx_unique_token only appears here",
            Kind::Note,
            "r",
            &[],
            "a",
            3,
        )
        .unwrap();

    let mut emb = Embedder::nomic_text().unwrap();
    let v = emb.embed_one(&rec.body).unwrap();
    let idx = MemoryIndex::open(paths.vectors_dir(), 768).await.unwrap();
    idx.upsert(&rec, &v).await.unwrap();

    let fts = Fts::open(paths.vectors_dir().join("fts.sqlite")).unwrap();
    fts.upsert(&rec.frontmatter.id, &rec.body).unwrap();
    drop(fts);

    // Query with a rare token; vector search may not bring it back near the
    // top, but BM25 will, and the fuser must surface the row.
    let q = emb.embed_one("zzzyx_unique_token").unwrap();
    let hits = search_memory_fused(
        &idx,
        &paths.vectors_dir().join("fts.sqlite"),
        &q,
        "zzzyx_unique_token",
        5,
        60.0,
    )
    .await
    .unwrap();
    assert!(
        hits.iter().any(|h| h.id == rec.frontmatter.id),
        "fused search dropped the lexical-only match"
    );
}

#[tokio::test]
async fn fused_search_degrades_to_vector_when_fts_missing() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();

    let store = MemoryStore::new(paths.clone());
    let rec = store
        .save("Use Postgres for analytics", Kind::Decision, "r", &[], "a", 3)
        .unwrap();
    let mut emb = Embedder::nomic_text().unwrap();
    let v = emb.embed_one(&rec.body).unwrap();
    let idx = MemoryIndex::open(paths.vectors_dir(), 768).await.unwrap();
    idx.upsert(&rec, &v).await.unwrap();

    let q = emb.embed_one("postgres analytics").unwrap();
    let hits = search_memory_fused(
        &idx,
        &paths.vectors_dir().join("missing.sqlite"),
        &q,
        "postgres analytics",
        5,
        60.0,
    )
    .await
    .unwrap();
    assert_eq!(hits[0].id, rec.frontmatter.id);
}
```

Append to `tests/retrieval.rs`:

```rust
mod fuse;
```

- [ ] **Step 3: Run tests, expect fail**

Run: `cargo nextest run --all-features -E 'test(/fused_search_/)'`
Expected: FAIL — `unresolved import`.

- [ ] **Step 4: Implement `search_memory_fused`**

Create `src/retrieval/fuse.rs`:

```rust
//! Dense + sparse retrieval fusion. Runs the vector path (`MemoryIndex`) and
//! the BM25 path (`retrieval::fts`) independently, RRF-fuses their ranked id
//! lists, then reifies `MemoryHit` rows for the top `limit` ids out of the
//! dense table. The dense table is treated as the canonical source for body
//! text and metadata so we never re-read the markdown file.

use std::collections::HashMap;
use std::path::Path;

use crate::index::{MemoryHit, MemoryIndex};
use crate::prelude::*;
use crate::retrieval::fts::search_fts_ids;
use crate::retrieval::rank::rrf_fuse;

/// Run vector + BM25 retrieval over the memory layer, fuse the rankings with
/// Reciprocal Rank Fusion, and return the top `limit` materialized hits.
///
/// `over_fetch_factor` is fixed at `4` so each underlying index returns
/// `limit * 4` candidates: enough overlap for fusion to act without blowing
/// up the SQL or vector query.
pub async fn search_memory_fused(
    idx: &MemoryIndex,
    fts_db: impl AsRef<Path>,
    query_emb: &[f32],
    query_text: &str,
    limit: usize,
    rrf_k: f32,
) -> Result<Vec<MemoryHit>> {
    let over = limit.saturating_mul(4).max(limit);

    let dense_hits = idx.search(query_emb, over).await?;
    let dense_ids: Vec<String> = dense_hits.iter().map(|h| h.id.clone()).collect();
    let sparse_ids = search_fts_ids(fts_db, query_text, over)?;

    let dense_ref: &[String] = &dense_ids;
    let sparse_ref: &[String] = &sparse_ids;
    let fused = rrf_fuse(&[dense_ref, sparse_ref], rrf_k);

    let by_id: HashMap<String, MemoryHit> =
        dense_hits.into_iter().map(|h| (h.id.clone(), h)).collect();

    let mut out = Vec::with_capacity(limit);
    for (id, score) in fused {
        if let Some(mut hit) = by_id.get(&id).cloned() {
            hit.score = score;
            out.push(hit);
            if out.len() == limit {
                break;
            }
        }
    }
    Ok(out)
}
```

Modify `src/retrieval/mod.rs`: add `pub mod fuse;` and `pub use fuse::search_memory_fused;`.

- [ ] **Step 5: Run tests, expect pass**

Run: `cargo nextest run --all-features -E 'test(/fused_search_/)'`
Expected: PASS — 2 tests.

- [ ] **Step 6: Commit**

```bash
git add src/retrieval/fuse.rs src/retrieval/mod.rs src/config/file.rs \
        tests/retrieval/fuse.rs tests/retrieval.rs
git commit -m "feat(retrieval): RRF-fuse dense and BM25 retrieval

Runs MemoryIndex vector search and FTS5 BM25 search in parallel-ready
shape, then RRF-fuses their ranked id lists. Lexical-only matches
(rare tokens, exact identifiers) survive fusion; missing FTS db
degrades gracefully to vector-only."
```

---

## Task 7: Wire embed-on-save + FTS-on-save into `cli/save.rs`

Both are best-effort, follow the same warn-and-swallow pattern as the existing graph upsert.

**Files:**
- Modify: `src/cli/save.rs`
- Test: `tests/cli/` (add `save_embed.rs`)

- [ ] **Step 1: Write failing test**

Create `tests/cli/save_embed.rs`:

```rust
use comemory::config::paths::Paths;
use comemory::index::{Embedder, Fts, MemoryIndex};
use comemory::memory::{Kind, MemoryStore};

use super::common;

#[tokio::test]
async fn save_writes_into_memory_index_and_fts() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();

    // Drive the CLI module directly so we don't shell out for every assert.
    let args = comemory::cli::save::Args {
        body: Some("Postgres analytics decision".into()),
        kind: Kind::Decision,
        repo: "r".into(),
        tags: String::new(),
        author: "a".into(),
        quality: 3,
    };
    comemory::cli::save::run(args, false, Some(paths.data_dir().to_path_buf()))
        .await
        .unwrap();

    // Memory store has the record.
    let store = MemoryStore::new(paths.clone());
    let listed = store.list().unwrap();
    assert_eq!(listed.len(), 1);
    let id = listed[0].frontmatter.id.clone();

    // Vector index has it.
    let idx = MemoryIndex::open(paths.vectors_dir(), 768).await.unwrap();
    let mut emb = Embedder::nomic_text().unwrap();
    let q = emb.embed_one("postgres analytics").unwrap();
    let hits = idx.search(&q, 5).await.unwrap();
    assert!(hits.iter().any(|h| h.id == id), "vector index missing save");

    // FTS has it.
    let fts = Fts::open(paths.vectors_dir().join("fts.sqlite")).unwrap();
    assert_eq!(fts.count().unwrap(), 1);
}
```

Add module wiring to `tests/cli.rs`:

```rust
mod save_embed;
```

(Confirm `tests/cli/` exists; if not, create `tests/cli/mod.rs` to declare `pub use super::common;` consistent with sibling test binaries.)

- [ ] **Step 2: Run test, expect fail**

Run: `cargo nextest run --all-features -E 'test(save_writes_into_memory_index_and_fts)'`
Expected: FAIL — `MemoryIndex::search` returns empty; FTS row count is 0.

- [ ] **Step 3: Update `cli::save::run`**

Add this block immediately after the existing `upsert_graph` call in `src/cli/save.rs`:

```rust
    // Best-effort dense embedding + FTS upsert. Either failure logs and is
    // swallowed: markdown remains the source of truth, and the user-facing
    // `comemory save` path must not fail just because LanceDB or SQLite
    // cannot open under the current data dir.
    if let Err(e) = upsert_indices(&paths, &rec).await {
        tracing::warn!("index upsert failed: {e}");
    }
```

Append helper at the bottom of `src/cli/save.rs`:

```rust
async fn upsert_indices(paths: &Paths, rec: &crate::memory::MemoryRecord) -> Result<()> {
    let idx = crate::index::MemoryIndex::open(paths.vectors_dir(), 768).await?;
    let mut emb = crate::index::Embedder::nomic_text()?;
    let v = emb.embed_one(&rec.body)?;
    idx.upsert(rec, &v).await?;

    let fts = crate::index::Fts::open(paths.vectors_dir().join("fts.sqlite"))?;
    fts.upsert(&rec.frontmatter.id, &rec.body)?;
    Ok(())
}
```

Add `use crate::index::{Embedder, Fts, MemoryIndex};` is **not** needed since the helper uses qualified paths to keep the existing import list short and the module under the 500-line cap.

- [ ] **Step 4: Run tests, expect pass**

Run: `cargo nextest run --all-features -E 'test(save_writes_into_memory_index_and_fts)'`
Expected: PASS.

- [ ] **Step 5: Run the full suite to confirm no regression**

Run: `bash scripts/check-all.sh`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/cli/save.rs tests/cli/save_embed.rs tests/cli.rs
git commit -m "feat(cli): embed memory body and write FTS row on save

Closes the v1.1 gap where MemoryIndex was populated only by
index-code. Both index writes are best-effort: markdown remains the
source of truth, so embedder/SQLite failures warn and are swallowed."
```

---

## Task 8: Switch `cli::search` to fused retrieval

Keep the route classifier and corrective-fallback observability; only the search call changes.

**Files:**
- Modify: `src/cli/search.rs`
- Test: `tests/cli/` (add `search_fused.rs`)

- [ ] **Step 1: Write failing test**

Create `tests/cli/search_fused.rs`:

```rust
use assert_cmd::Command;
use comemory::config::paths::Paths;
use serde_json::Value;

use super::common;

#[test]
fn search_surfaces_lexical_only_match() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());

    let mut save = Command::cargo_bin("comemory").unwrap();
    save.env("COMEMORY_DATA_DIR", paths.data_dir())
        .arg("save")
        .arg("zzzyx_unique_token: a rare lexical marker")
        .arg("--kind")
        .arg("note")
        .arg("--repo")
        .arg("r");
    save.assert().success();

    let out = Command::cargo_bin("comemory")
        .unwrap()
        .env("COMEMORY_DATA_DIR", paths.data_dir())
        .arg("--json")
        .arg("search")
        .arg("zzzyx_unique_token")
        .arg("--limit")
        .arg("5")
        .output()
        .unwrap();
    assert!(out.status.success(), "search exited non-zero");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let hits = v["hits"].as_array().unwrap();
    assert!(!hits.is_empty(), "fused search returned no hits");
    let first = hits[0]["snippet"].as_str().unwrap();
    assert!(first.contains("zzzyx_unique_token"));
}
```

Add to `tests/cli.rs`:

```rust
mod search_fused;
```

- [ ] **Step 2: Run test, expect fail**

Run: `cargo nextest run --all-features -E 'test(search_surfaces_lexical_only_match)'`
Expected: FAIL — vector-only path may rank the rare-token hit below the threshold.

- [ ] **Step 3: Wire fused search into `cli::search::run`**

Edit `src/cli/search.rs` — replace the `let hits = search_memory(...)` line with:

```rust
    let hits = comemory::retrieval::fuse::search_memory_fused(
        &idx,
        paths.vectors_dir().join("fts.sqlite"),
        &q,
        &a.query,
        a.limit,
        60.0,
    )
    .await?;
```

Replace `use crate::retrieval::hybrid::search_memory;` with the fuse module path import:

```rust
use crate::retrieval::fuse::search_memory_fused;
```

…then call `search_memory_fused(...)` instead of `comemory::retrieval::...` to keep the local file consistent. (Use whichever import style matches the rest of `src/cli/` — `use` + bare call is the existing convention.)

Drop the now-unused `search_memory` import.

- [ ] **Step 4: Run tests, expect pass**

Run: `cargo nextest run --all-features -E 'test(search_surfaces_lexical_only_match)'`
Expected: PASS.

- [ ] **Step 5: Run full gate**

Run: `bash scripts/check-all.sh`
Expected: PASS (formatting, clippy, placement, no-bypass, module-size, tests-mirror, typos).

- [ ] **Step 6: Commit**

```bash
git add src/cli/search.rs tests/cli/search_fused.rs tests/cli.rs
git commit -m "feat(cli): search uses RRF-fused dense+BM25 retrieval

Lexical-only matches (rare tokens, exact identifiers) now survive
ranking even when their dense similarity is below threshold. Route
classifier and corrective-fallback signal are unchanged."
```

---

## Task 9: Criterion bench scaffolding

**Files:**
- Modify: `Cargo.toml`
- Create: `benches/common.rs`, `benches/save.rs`, `benches/retrieval.rs`

- [ ] **Step 1: Add criterion dev-dep and bench entries**

Append to `Cargo.toml` (`[dev-dependencies]`):

```toml
criterion = { version = "0.5", default-features = false, features = ["html_reports"] }
```

Append at the end of `Cargo.toml`:

```toml
[[bench]]
name = "save"
harness = false

[[bench]]
name = "retrieval"
harness = false
```

- [ ] **Step 2: Create shared bench fixture**

Create `benches/common.rs`:

```rust
//! Shared bench harness: deterministic temp data dir + a seeded corpus so
//! every bench compares apples to apples.

use std::path::PathBuf;

use comemory::config::paths::Paths;
use comemory::index::{Embedder, Fts, MemoryIndex};
use comemory::memory::{Kind, MemoryStore};
use tempfile::TempDir;

pub struct Fixture {
    pub _tmp: TempDir,
    pub paths: Paths,
}

pub fn fixture() -> Fixture {
    let tmp = TempDir::new().expect("tempdir");
    let paths = Paths::new(tmp.path().join(".comemory"));
    paths.ensure_dirs().expect("ensure_dirs");
    Fixture { _tmp: tmp, paths }
}

pub async fn seed(paths: &Paths, n: usize) -> Vec<String> {
    let store = MemoryStore::new(paths.clone());
    let mut emb = Embedder::nomic_text().expect("embedder");
    let idx = MemoryIndex::open(paths.vectors_dir(), 768)
        .await
        .expect("memory index");
    let fts = Fts::open(paths.vectors_dir().join("fts.sqlite")).expect("fts");
    let bodies: Vec<String> = (0..n)
        .map(|i| format!("seed body {i}: postgres analytics token_{i}"))
        .collect();
    let mut ids = Vec::with_capacity(n);
    for body in &bodies {
        let rec = store
            .save(body, Kind::Note, "bench", &[], "bench", 3)
            .expect("save");
        let v = emb.embed_one(&rec.body).expect("embed");
        idx.upsert(&rec, &v).await.expect("upsert");
        fts.upsert(&rec.frontmatter.id, &rec.body).expect("fts upsert");
        ids.push(rec.frontmatter.id);
    }
    ids
}

#[allow(dead_code)]
pub fn data_dir(f: &Fixture) -> PathBuf {
    f.paths.data_dir().to_path_buf()
}
```

NOTE: `#[allow(dead_code)]` is permitted because `benches/` is outside `src/` and `scripts/`, so the no-bypass-check ignores it. Confirm in step 6.

- [ ] **Step 3: Create save bench**

Create `benches/save.rs`:

```rust
//! Bench: end-to-end `comemory save` cost (markdown write + dense embed +
//! FTS insert). Reports mean + p99 in nanoseconds via criterion.

use comemory::index::{Embedder, Fts, MemoryIndex};
use comemory::memory::{Kind, MemoryStore};
use criterion::{criterion_group, criterion_main, Criterion};
use tokio::runtime::Runtime;

mod common;

fn bench_save(c: &mut Criterion) {
    let rt = Runtime::new().expect("rt");
    c.bench_function("save_end_to_end", |b| {
        b.to_async(&rt).iter_with_setup(
            || {
                let fx = common::fixture();
                let body = "Bench memory: postgres analytics decision token";
                (fx, body.to_string())
            },
            |(fx, body)| async move {
                let store = MemoryStore::new(fx.paths.clone());
                let rec = store
                    .save(&body, Kind::Note, "bench", &[], "bench", 3)
                    .expect("save");
                let idx = MemoryIndex::open(fx.paths.vectors_dir(), 768)
                    .await
                    .expect("idx");
                let mut emb = Embedder::nomic_text().expect("emb");
                let v = emb.embed_one(&rec.body).expect("embed");
                idx.upsert(&rec, &v).await.expect("upsert");
                let fts = Fts::open(fx.paths.vectors_dir().join("fts.sqlite")).expect("fts");
                fts.upsert(&rec.frontmatter.id, &rec.body).expect("fts upsert");
            },
        );
    });
}

criterion_group!(benches, bench_save);
criterion_main!(benches);
```

- [ ] **Step 4: Create retrieval bench**

Create `benches/retrieval.rs`:

```rust
//! Bench: vector-only `search_memory` vs RRF-fused `search_memory_fused` over
//! a 100-row seeded corpus. Reports the latency delta directly.

use comemory::index::{Embedder, MemoryIndex};
use comemory::retrieval::fuse::search_memory_fused;
use comemory::retrieval::hybrid::search_memory;
use criterion::{criterion_group, criterion_main, Criterion};
use tokio::runtime::Runtime;

mod common;

fn bench_search(c: &mut Criterion) {
    let rt = Runtime::new().expect("rt");
    let fx = common::fixture();
    rt.block_on(async {
        let _ = common::seed(&fx.paths, 100).await;
    });
    let mut emb = Embedder::nomic_text().expect("emb");
    let q_vec = emb.embed_one("postgres analytics").expect("embed");
    let q_text = "postgres analytics".to_string();
    let idx = rt.block_on(MemoryIndex::open(fx.paths.vectors_dir(), 768)).expect("idx");
    let fts_db = fx.paths.vectors_dir().join("fts.sqlite");

    c.bench_function("search_vector_only", |b| {
        b.to_async(&rt)
            .iter(|| async { search_memory(&idx, &q_vec, 12, 0.55).await.expect("search") });
    });

    c.bench_function("search_fused_rrf", |b| {
        b.to_async(&rt).iter(|| async {
            search_memory_fused(&idx, &fts_db, &q_vec, &q_text, 12, 60.0)
                .await
                .expect("fused")
        });
    });
}

criterion_group!(benches, bench_search);
criterion_main!(benches);
```

- [ ] **Step 5: Build benches to confirm wiring**

Run: `cargo bench --no-run --all-features`
Expected: clean build, prints `Compiling comemory v0.1.0 ...` and `Executable benches/save-<hash>` lines.

- [ ] **Step 6: Confirm gates still pass**

Run: `bash scripts/check-all.sh`
Expected: PASS. Note: `scripts/no-bypass-check.sh` is scoped to `src/` + `scripts/`, so `#[allow(dead_code)]` in `benches/` is allowed; verify by re-reading the script if uncertain.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml benches/common.rs benches/save.rs benches/retrieval.rs
git commit -m "bench: criterion harness for save and retrieval

save_end_to_end: markdown + embed + memory-index upsert + FTS insert.
search_vector_only vs search_fused_rrf over a 100-row corpus: lets us
defend the RRF latency budget as the fused path becomes the default."
```

---

## Task 10: `scripts/bench.sh` + `just bench` + docs

**Files:**
- Create: `scripts/bench.sh`, `docs/bench/README.md`
- Modify: `justfile`, `README.md`

- [ ] **Step 1: Write `scripts/bench.sh`**

Create `scripts/bench.sh`:

```bash
#!/usr/bin/env bash
# Reproducible bench runner. Pins thread count and warmup so successive runs
# are comparable. Output lands in docs/bench/latest.md plus the criterion
# HTML report under target/criterion/.

set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

OUT="docs/bench"
mkdir -p "$OUT"

echo "# comemory bench results" > "$OUT/latest.md"
echo "" >> "$OUT/latest.md"
echo "Rust: $(rustc --version)" >> "$OUT/latest.md"
echo "Host: $(uname -m) $(uname -s)" >> "$OUT/latest.md"
echo "Run at: $(date -u +%FT%TZ)" >> "$OUT/latest.md"
echo "" >> "$OUT/latest.md"
echo '```' >> "$OUT/latest.md"
RUST_LOG=warn cargo bench --all-features 2>&1 | tee -a "$OUT/latest.md"
echo '```' >> "$OUT/latest.md"

echo "wrote $OUT/latest.md"
```

Make it executable:

```bash
chmod +x scripts/bench.sh
```

- [ ] **Step 2: Add `just bench` recipe**

Append to `justfile`:

```make
# Run criterion benches and write a Markdown report to docs/bench/latest.md.
bench:
    bash scripts/bench.sh
```

- [ ] **Step 3: Write `docs/bench/README.md`**

Create `docs/bench/README.md`:

```markdown
# Benchmarks

Run `just bench` to regenerate `docs/bench/latest.md`. The harness is criterion
0.5; results include mean, median, and p99 latency.

## What we track

- `save_end_to_end` — `comemory save` cost: markdown write + nomic embedding +
  `MemoryIndex::upsert` + FTS5 insert. Watch this when changing the embed or
  upsert path.
- `search_vector_only` — `retrieval::hybrid::search_memory` baseline.
- `search_fused_rrf` — `retrieval::fuse::search_memory_fused` (dense ⊕ BM25
  with Reciprocal Rank Fusion). The delta vs `search_vector_only` is the
  latency cost of fusion + the FTS5 round-trip.

## Reproducibility

Numbers vary across hardware, ONNX runtime version, and cold-vs-warm fastembed
model cache. Re-run the bench on the same host before comparing.
```

- [ ] **Step 4: Update `README.md`**

In `README.md`, find the "Known v1.1 gaps" section and remove the line about memory bodies not being embedded on save. Add a "Benchmarks" subsection below it pointing at `docs/bench/`:

```markdown
### Benchmarks

`just bench` runs the criterion harness and writes `docs/bench/latest.md`.
The save and retrieval suites cover the embed-on-save path and the RRF-fused
dense+BM25 search introduced in v1.1.
```

- [ ] **Step 5: Run gates one more time**

Run: `bash scripts/check-all.sh`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add scripts/bench.sh justfile docs/bench/README.md README.md
git commit -m "docs(bench): wire just bench + reproducible runner

scripts/bench.sh emits docs/bench/latest.md with host + toolchain
metadata so successive runs are comparable. Drops the v1.1 embed-on-save
gap from the README since this bundle closes it."
```

---

## Task 11: Final verification + plan-complete commit

- [ ] **Step 1: Full gate**

Run: `bash scripts/check-all.sh && cargo nextest run --all-features`
Expected: PASS.

- [ ] **Step 2: Smoke-run the bench**

Run: `just bench`
Expected: `docs/bench/latest.md` exists and contains `search_vector_only` and `search_fused_rrf` results.

- [ ] **Step 3: Confirm CLI behaviour by hand**

```bash
export COMEMORY_DATA_DIR=$(mktemp -d)/.comemory
cargo run -- save "smoke: zzzyx_unique_token appears once" --kind note --repo smoke
cargo run -- search "zzzyx_unique_token" --limit 3 --json
```

Expected: search JSON has a hit whose `snippet` contains `zzzyx_unique_token`.

- [ ] **Step 4: Commit the bench output if reproducible**

```bash
git add docs/bench/latest.md
git commit -m "docs(bench): initial reference run

Baseline numbers captured on the dev host; treat as approximate.
Re-run \`just bench\` on the same hardware before comparing."
```

(Skip if `latest.md` is host-specific and you'd rather keep it gitignored — in that case append `docs/bench/latest.md` to `.gitignore` and commit that change instead.)

---

## Self-Review Notes

- **Spec coverage:** (1) embed-on-save → Task 7. (2) BM25 hybrid + RRF → Tasks 1–6, 8. (6) criterion benches → Tasks 9–10. All three covered.
- **Module-size budget:** `src/cli/save.rs` grows by ~20 lines (currently ~120) — well under 500. `src/index/fts.rs` ≤200. `src/retrieval/fuse.rs` ≤150. No file approaches the 500-line cap.
- **No-bypass-check:** every `Result` path uses the crate `Error::Other(...)` mapping; no `.unwrap()` in `src/`. Test fixtures and benches use `.unwrap()`/`expect("..")` which are permitted outside `src/` + `scripts/`.
- **Tests-mirror-check:** every new `src/` file has a matching test file (`tests/index/fts.rs`, `tests/retrieval/fts.rs`, `tests/retrieval/fuse.rs`).
- **DRY:** RRF lives in one place (`retrieval::rank::rrf_fuse`); FTS lives in one place (`index::fts`); `retrieval::fts` is a 1-screen wrapper that exists only so the retrieval layer never leaks a SQLite type to callers.
