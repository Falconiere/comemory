//! SQLite-backed stats store: schema bootstrap and connection accessors.
//!
//! Holds three tables: `retrieval_log` (per-query log), `feedback` (per-memory
//! used/irrelevant counters), and `repo_marker` (per-repo last-indexed head).
//! Schema is idempotent (`CREATE TABLE IF NOT EXISTS`) so callers can open
//! existing databases without migration.

use std::path::Path;

use rusqlite::Connection;

use crate::prelude::*;

/// Owns a SQLite connection for the stats database. Open on startup, reuse for
/// the process lifetime.
pub struct StatsDb {
    conn: Connection,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS retrieval_log(
  query_id     TEXT PRIMARY KEY,
  query        TEXT NOT NULL,
  returned_ids TEXT NOT NULL,
  at           TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS feedback(
  memory_id        TEXT PRIMARY KEY,
  used_count       INTEGER NOT NULL DEFAULT 0,
  irrelevant_count INTEGER NOT NULL DEFAULT 0,
  last_used        TEXT
);
CREATE TABLE IF NOT EXISTS repo_marker(
  repo            TEXT PRIMARY KEY,
  last_head       TEXT,
  last_indexed_at TEXT
);
"#;

impl StatsDb {
    /// Open (or create) the stats database at `path`, ensuring the parent
    /// directory exists and the schema is applied.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path.as_ref()).map_err(|e| Error::Other(e.to_string()))?;
        conn.execute_batch(SCHEMA)
            .map_err(|e| Error::Other(e.to_string()))?;
        Ok(Self { conn })
    }

    /// Borrow the underlying connection (read-only access).
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Borrow the underlying connection mutably (for transactions).
    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}
