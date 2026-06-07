//! SQLite-backed stats store: schema bootstrap and connection accessors.
//!
//! Holds three tables: `retrieval_log` (per-query log), `feedback` (per-memory
//! used/irrelevant counters), and `repo_marker` (per-repo last-indexed head).
//! Schema is idempotent (`CREATE TABLE IF NOT EXISTS`) so callers can open
//! existing databases without migration.

use std::path::Path;

use rusqlite::Connection;
use time::format_description::well_known::Iso8601;
use time::OffsetDateTime;

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
CREATE TABLE IF NOT EXISTS index_failures(
  id    INTEGER PRIMARY KEY AUTOINCREMENT,
  ts    TEXT NOT NULL,
  error TEXT NOT NULL
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

    /// Append a row to `index_failures` recording a swallowed indexing
    /// failure. Callers feed this from `comemory save` when the dense embed +
    /// FTS upsert is skipped via `tracing::warn!`-and-continue, so operators
    /// running on a read-only mount (or with a broken ONNX cache) have a
    /// durable signal instead of a vanished log line.
    ///
    /// `when` is the wall-clock timestamp of the failure (ISO 8601 in UTC);
    /// the caller passes it explicitly so tests can pin a deterministic value.
    /// `error` is the stringified `Display` of the original error.
    pub fn record_index_failure(&self, when: OffsetDateTime, error: &str) -> Result<()> {
        let ts = when
            .to_offset(time::UtcOffset::UTC)
            .format(&Iso8601::DEFAULT)
            .map_err(|e| Error::Other(e.to_string()))?;
        self.conn
            .execute(
                "INSERT INTO index_failures(ts, error) VALUES (?1, ?2)",
                rusqlite::params![ts, error],
            )
            .map_err(|e| Error::Other(e.to_string()))?;
        Ok(())
    }

    /// Number of rows in `index_failures`. Surfaced by `comemory doctor` and
    /// tests; saturates at `usize::MAX` because the underlying count is
    /// signed in SQLite and we clamp to 0 on negative results.
    pub fn index_failure_count(&self) -> Result<usize> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM index_failures", [], |r| r.get(0))
            .map_err(|e| Error::Other(e.to_string()))?;
        Ok(n.max(0) as usize)
    }

    /// Most recent `(ts, error)` row in `index_failures`, or `None` when the
    /// table is empty. The timestamp is the ISO 8601 string recorded by
    /// [`Self::record_index_failure`]; the error is the original `Display`
    /// payload.
    pub fn last_index_failure(&self) -> Result<Option<(String, String)>> {
        match self.conn.query_row(
            "SELECT ts, error FROM index_failures ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        ) {
            Ok(row) => Ok(Some(row)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(Error::Other(format!("last_index_failure: {e}"))),
        }
    }
}
