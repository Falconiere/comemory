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

    /// BM25 search. Empty / whitespace-only queries short-circuit to an empty
    /// result so callers don't have to filter them. The raw query string is
    /// passed straight to FTS5 — callers that want phrase or column filters
    /// can pass standard FTS5 syntax.
    ///
    /// Malformed MATCH expressions (column qualifiers on missing columns,
    /// trailing operators, bare apostrophes, etc.) are downgraded to an
    /// empty result with a `tracing::debug!` note. FTS5 surfaces these as
    /// `SQLITE_ERROR` strings during row iteration; we never want a typo in
    /// the user query to abort the fused retrieval pipeline.
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
        let rows = match stmt.query_map(rusqlite::params![query, limit as i64], |row| {
            let id: String = row.get(0)?;
            let raw: f64 = row.get(1)?;
            Ok(FtsHit { id, score: -raw })
        }) {
            Ok(r) => r,
            Err(e) => {
                if is_fts5_parse_error(&e) {
                    tracing::debug!("fts5 query parse error: {e}; treating as empty");
                    return Ok(Vec::new());
                }
                return Err(Error::Other(e.to_string()));
            }
        };
        let mut out = Vec::new();
        for r in rows {
            match r {
                Ok(hit) => out.push(hit),
                Err(e) => {
                    if is_fts5_parse_error(&e) {
                        tracing::debug!("fts5 query parse error: {e}; treating as empty");
                        return Ok(Vec::new());
                    }
                    return Err(Error::Other(e.to_string()));
                }
            }
        }
        Ok(out)
    }
}

/// Best-effort detection of an FTS5 MATCH-expression parse error. FTS5 reports
/// these as `SQLITE_ERROR` (rusqlite's `SqliteFailure` variant) prefixed with
/// `"fts5:"`. Tokenizer parse failures (e.g. unbalanced quote) sometimes
/// surface as `"syntax error near \"<token>\""` without the prefix on older
/// SQLite builds, so we keep that keeper.
///
/// `"no such column"` is **not** matched here: it indicates a genuine schema
/// mismatch (or a column-qualified MATCH expression against a missing column)
/// and must propagate to the caller as `Err` instead of silently degrading
/// to an empty result.
fn is_fts5_parse_error(e: &rusqlite::Error) -> bool {
    let s = e.to_string().to_lowercase();
    s.starts_with("fts5:") || s.contains("fts5") || s.contains("syntax error")
}

/// One BM25 hit. `score` is the negated `bm25()` value (FTS5 returns negative
/// scores where smaller means more relevant); we flip the sign so callers can
/// sort descending uniformly with `MemoryHit::score`.
#[derive(Debug, Clone)]
pub struct FtsHit {
    /// Memory id stored in the `UNINDEXED` payload column.
    pub id: String,
    /// `-bm25()` so higher is more relevant.
    pub score: f64,
}
