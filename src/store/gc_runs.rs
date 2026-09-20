//! `gc_runs` row insert plus the newest-row read — one row per `comemory
//! gc` sweep, recording its removal counts and reclaimed bytes for the v14
//! console-history table (`migrations/0014_v14_console.sql`).
//! [`newest`] backs `GET /api/v1/gc/policy`'s `last_run` / `last_run_at`.

use rusqlite::Connection;
use serde::Serialize;

use super::{
    orm,
    schema_history::{GcRuns, gc_runs as col},
};
use crate::prelude::*;

/// Insert parameters for one completed sweep, bundled into a struct rather
/// than eight positional arguments (`clippy::too_many_arguments`), the same
/// shape [`super::index_runs::NewIndexRun`] uses.
pub struct NewGcRun<'a> {
    /// 16-hex run id (`store::random_id::random_hex`).
    pub id: &'a str,
    /// Pre-rendered ISO-8601 UTC timestamp (`store::memory_row::iso_format`).
    pub at: &'a str,
    /// Trashed memory files hard-deleted.
    pub removed: u64,
    /// `retrieval_log` rows evicted.
    pub log_rows: u64,
    /// `feedback_events` rows evicted.
    pub event_rows: u64,
    /// Bytes reclaimed from the trash sweep.
    pub bytes_freed: u64,
    /// `activity_log` rows evicted.
    pub activity_rows: u64,
}

/// Insert one `gc_runs` row for a completed sweep. A single `INSERT` with no
/// read-modify-write race: every field is caller-computed.
pub fn insert(conn: &Connection, run: &NewGcRun<'_>) -> Result<()> {
    orm::execute(
        conn,
        GcRuns::insert()
            .set(&col::id, run.id)
            .set(&col::at, run.at)
            .set(&col::removed, clamp(run.removed))
            .set(&col::log_rows, clamp(run.log_rows))
            .set(&col::event_rows, clamp(run.event_rows))
            .set(&col::bytes_freed, clamp(run.bytes_freed))
            .set(&col::activity_rows, clamp(run.activity_rows))
            .to_sql(),
    )?;
    Ok(())
}

/// Saturate a `u64` count into SQLite's `i64` column type.
fn clamp(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
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
/// lexicographic order chronological (see `maintenance::gc::sweep_learning`'s doc),
/// and the `rowid` tie-break returns the LATER-INSERTED row when two sweeps
/// land in the same nanosecond. Ids are random hex, so ordering by id would
/// be deterministic but arbitrary — it would sometimes answer with the
/// earlier sweep.
pub fn newest(conn: &Connection) -> Result<Option<GcRunRow>> {
    orm::query_optional(
        conn,
        GcRuns::select()
            .columns_typed(&[
                &col::id,
                &col::at,
                &col::removed,
                &col::log_rows,
                &col::event_rows,
                &col::bytes_freed,
            ])
            .order_by(col::at.desc())
            .order_by(toolu_orm::core::expr::OrderBy::alias_desc("rowid"))
            .limit(1)
            .to_sql(),
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
}

/// A stored counter as `u64`, clamping a (impossible-in-practice) negative
/// value to `0` — see [`GcRunRow`]'s doc.
fn to_count(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

#[cfg(test)]
#[path = "tests/gc_runs.rs"]
mod tests;
