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

use super::{
    orm,
    schema_history::{IndexFailures, index_failures as c},
};
use rusqlite::Connection;
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
    orm::execute(
        conn,
        IndexFailures::insert()
            .set(&c::ts, ts)
            .set(&c::error, error)
            .to_sql(),
    )?;
    Ok(())
}

/// Number of rows in `index_failures`. Surfaced by `comemory doctor` and
/// tests. SQLite's `COUNT(*)` is signed, so a (structurally impossible)
/// negative result clamps to 0; the upper bound is therefore `i64::MAX`
/// widened to `usize`, not `usize::MAX`.
pub fn count(conn: &Connection) -> Result<usize> {
    let n: i64 = orm::query_one(conn, IndexFailures::select().to_count_sql(), |r| r.get(0))?;
    Ok(n.max(0) as usize)
}

/// Most recent `(ts, error)` row, or `None` when the table is empty. The
/// timestamp is the ISO 8601 string written by [`record`]; the error is the
/// original `Display` payload.
pub fn latest(conn: &Connection) -> Result<Option<(String, String)>> {
    orm::query_optional(
        conn,
        IndexFailures::select()
            .columns_typed(&[&c::ts, &c::error])
            .order_by(c::id.desc())
            .limit(1)
            .to_sql(),
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
}

#[cfg(test)]
#[path = "tests/index_failures.rs"]
mod tests;
