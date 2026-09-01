//! `gc_runs` row insert — one row per `comemory gc` sweep, recording its
//! removal counts and reclaimed bytes for the v14 console-history table
//! (`src/store/sql/0014_v14_console.sql`).

use rusqlite::Connection;

use crate::prelude::*;

/// Insert one `gc_runs` row for a completed sweep. `id` is caller-generated
/// (16 lowercase-hex chars via [`crate::store::random_id::random_hex`]) so
/// the write is a single `INSERT` with no read-modify-write race; `at` is a
/// pre-rendered ISO-8601 timestamp (`store::memory_row::iso_format`).
pub fn insert(
    conn: &Connection,
    id: &str,
    at: &str,
    removed: u64,
    log_rows: u64,
    event_rows: u64,
    bytes_freed: u64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO gc_runs(id, at, removed, log_rows, event_rows, bytes_freed) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            id,
            at,
            i64::try_from(removed).unwrap_or(i64::MAX),
            i64::try_from(log_rows).unwrap_or(i64::MAX),
            i64::try_from(event_rows).unwrap_or(i64::MAX),
            i64::try_from(bytes_freed).unwrap_or(i64::MAX),
        ],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/gc_runs.rs"]
mod tests;
