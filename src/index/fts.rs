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

    /// Return the number of rows currently indexed in `memory_fts`. Useful
    /// for health checks and progress reporting; upsert/search land in the
    /// next tasks.
    pub fn row_count(&self) -> Result<u64> {
        self.conn
            .query_row("SELECT count(*) FROM memory_fts", [], |r| {
                r.get::<_, u64>(0)
            })
            .map_err(|e| Error::Other(e.to_string()))
    }
}
