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
}
