//! `index_failures` row CRUD: an append-only log of swallowed indexing
//! failures (e.g. a skipped dense-embed/FTS upsert on a read-only mount).
//!
//! This module owns the whole concern — the SQL, the ISO 8601 (UTC)
//! timestamp the rows are keyed by, and the clamp that turns SQLite's signed
//! `COUNT(*)` into a `usize`. It used to share it with the stats handle,
//! whose three delegating methods were the last fragment the store-layer
//! chokepoint work left behind; #173 finished the move rather than carrying
//! index bookkeeping into [`crate::domains::learning`], which owns feedback
//! and evaluation and has nothing to do with indexing.

use rusqlite::{Connection, OptionalExtension, params};
use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;

use crate::prelude::*;

/// Append one row recording a swallowed indexing failure. Callers feed this
/// from `comemory save` when the dense embed + FTS upsert is skipped via
/// `tracing::warn!`-and-continue, so operators running on a read-only mount
/// (or with a broken ONNX cache) have a durable signal instead of a vanished
/// log line.
///
/// `when` is the wall-clock timestamp of the failure, stored as ISO 8601 in
/// UTC; the caller passes it explicitly so tests can pin a deterministic
/// value. `error` is the stringified `Display` of the original error.
pub fn record(conn: &Connection, when: OffsetDateTime, error: &str) -> Result<()> {
    let ts = when
        .to_offset(time::UtcOffset::UTC)
        .format(&Iso8601::DEFAULT)
        .map_err(|e| Error::Other(e.to_string()))?;
    conn.execute(
        "INSERT INTO index_failures(ts, error) VALUES (?1, ?2)",
        params![ts, error],
    )?;
    Ok(())
}

/// Number of rows in `index_failures`. Surfaced by `comemory doctor` and
/// tests; saturates at `usize::MAX` because the underlying count is signed
/// in SQLite and we clamp to 0 on negative results.
pub fn count(conn: &Connection) -> Result<usize> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM index_failures", [], |r| r.get(0))?;
    Ok(n.max(0) as usize)
}

/// Most recent `(ts, error)` row, or `None` when the table is empty. The
/// timestamp is the ISO 8601 string written by [`record`]; the error is the
/// original `Display` payload.
pub fn latest(conn: &Connection) -> Result<Option<(String, String)>> {
    let row = conn
        .query_row(
            "SELECT ts, error FROM index_failures ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?;
    Ok(row)
}

#[cfg(test)]
#[path = "tests/index_failures.rs"]
mod tests;
