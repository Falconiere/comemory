//! `gc_runs` row insert plus the newest-row read — one row per `comemory
//! gc` sweep, recording its removal counts and reclaimed bytes for the v14
//! console-history table (`src/store/sql/0014_v14_console.sql`).
//! [`newest`] backs `GET /api/v1/gc/policy`'s `last_run` / `last_run_at`.

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

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

/// One `gc_runs` row, as the console reads it back. Counters are stored as
/// SQLite INTEGERs (`i64`) and surfaced as `u64` — a negative value is
/// impossible for a count, so an out-of-range read clamps to `0` rather
/// than failing a policy read over a corrupt row.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct GcRunRow {
    /// The run's 16-hex id.
    pub id: String,
    /// ISO-8601 UTC timestamp of the sweep.
    pub at: String,
    /// Trashed memories hard-deleted by that run.
    pub removed: u64,
    /// `retrieval_log` rows evicted.
    pub log_rows: u64,
    /// `feedback_events` rows evicted.
    pub event_rows: u64,
    /// Bytes reclaimed from the trash directory.
    pub bytes_freed: u64,
}

/// The most recent `gc_runs` row, or `None` when `gc` has never run.
///
/// Ordered by `at DESC, rowid DESC`: every `at` is written through
/// `store::memory_row::iso_format`, whose fixed-width rendering makes
/// lexicographic order chronological (see `api::gc::sweep_learning`'s doc),
/// and the `rowid` tie-break returns the LATER-INSERTED row when two sweeps
/// land in the same nanosecond. Ids are random hex, so ordering by id would
/// be deterministic but arbitrary — it would sometimes answer with the
/// earlier sweep.
pub fn newest(conn: &Connection) -> Result<Option<GcRunRow>> {
    let row = conn
        .query_row(
            "SELECT id, at, removed, log_rows, event_rows, bytes_freed FROM gc_runs \
              ORDER BY at DESC, rowid DESC LIMIT 1",
            [],
            |r| {
                Ok(GcRunRow {
                    id: r.get(0)?,
                    at: r.get(1)?,
                    removed: to_count(r.get(2)?),
                    log_rows: to_count(r.get(3)?),
                    event_rows: to_count(r.get(4)?),
                    bytes_freed: to_count(r.get(5)?),
                })
            },
        )
        .optional()?;
    Ok(row)
}

/// A stored counter as `u64`, clamping a (impossible-in-practice) negative
/// value to `0` — see [`GcRunRow`]'s doc.
fn to_count(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

#[cfg(test)]
#[path = "tests/gc_runs.rs"]
mod tests;
