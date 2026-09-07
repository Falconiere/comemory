//! `index_failures` row CRUD: an append-only log of swallowed indexing
//! failures (e.g. a skipped dense-embed/FTS upsert on a read-only mount),
//! written and read back through [`crate::stats::sqlite::StatsDb`].

use rusqlite::{Connection, OptionalExtension, params};

use crate::prelude::*;

/// Append one `(ts, error)` row. `ts` is a pre-formatted ISO 8601 (UTC)
/// string; `error` is the stringified `Display` of the original error.
pub(crate) fn insert(conn: &Connection, ts: &str, error: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO index_failures(ts, error) VALUES (?1, ?2)",
        params![ts, error],
    )?;
    Ok(())
}

/// Raw row count. SQLite's `COUNT(*)` is signed; the caller clamps a
/// (structurally impossible) negative result rather than this helper
/// inventing a `usize` return type for a table that can never hold one.
pub(crate) fn count(conn: &Connection) -> Result<i64> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM index_failures", [], |r| r.get(0))?;
    Ok(n)
}

/// Most recent `(ts, error)` row, or `None` when the table is empty.
pub(crate) fn latest(conn: &Connection) -> Result<Option<(String, String)>> {
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
