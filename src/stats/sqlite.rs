//! SQLite-backed stats store: opens `comemory.db` via the shared
//! [`crate::store::connection::open`] so all data lands in the single v0.2
//! database (spec §4: one file).
//!
//! Three tables live here: `retrieval_log` (per-query log), `repo_marker`
//! (per-repo last-indexed head), and `index_failures` (swallowed indexing
//! errors). The `feedback` table was always in `0002_v2_tables.sql`; the
//! remaining tables were added in migration `0003_stats_tables`.

use rusqlite::Connection;
use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;

use crate::prelude::*;
use crate::store::connection;

/// Owns a SQLite connection to `comemory.db` for stats operations.
pub struct StatsDb {
    conn: Connection,
}

impl StatsDb {
    /// Open (or create) `comemory.db` at `path`, running all pending
    /// migrations so the stats tables are guaranteed to exist. Creates the
    /// parent directory on demand so callers do not need to invoke
    /// [`crate::config::paths::Paths::ensure_dirs`] first.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = connection::open(path)?;
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
        self.conn.execute(
            "INSERT INTO index_failures(ts, error) VALUES (?1, ?2)",
            rusqlite::params![ts, error],
        )?;
        Ok(())
    }

    /// Number of rows in `index_failures`. Surfaced by `comemory doctor` and
    /// tests; saturates at `usize::MAX` because the underlying count is
    /// signed in SQLite and we clamp to 0 on negative results.
    pub fn index_failure_count(&self) -> Result<usize> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM index_failures", [], |r| r.get(0))?;
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
            Err(e) => Err(Error::Sqlite(e)),
        }
    }
}
