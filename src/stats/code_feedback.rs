//! Per-symbol code feedback counters: `used` and `irrelevant`.
//!
//! Code-side sibling of [`crate::stats::feedback`] (read that first — the
//! shapes deliberately mirror each other). Each `code_feedback` row tracks
//! one `code_symbols` rowid; provenance lands in the shared
//! `feedback_events` table tagged `target_kind = 'code'`.
//!
//! Column-name wart: `feedback_events.memory_id` predates code targets, so
//! for `target_kind = 'code'` rows it carries the **text-encoded symbol id**
//! (e.g. symbol `42` → `'42'`). Readers must filter on `target_kind` before
//! interpreting the column — `eval::golden::harvest` and `eval::mine` do.
//!
//! The upsert/insert SQL intentionally parallels `feedback.rs` rather than
//! sharing a helper: the tables differ in name, key column, and key type,
//! and a generic helper parameterized on table name would be stringly-typed
//! overkill.

use rusqlite::Connection;
use time::OffsetDateTime;

use crate::prelude::*;
use crate::stats::sqlite::StatsDb;
use crate::store::memory_row;

/// Upsert the `used` side of the per-symbol counter row: insert with
/// `used_count = 1` or bump the existing count, refreshing `last_used` to
/// `now` either way. Mirrors [`crate::stats::feedback::upsert_used`].
/// Composed by [`record_code_with_provenance`] so the UPSERT SQL exists
/// exactly once.
pub(crate) fn upsert_code_used(conn: &Connection, id: i64, now: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO code_feedback(symbol_id, used_count, irrelevant_count, last_used)
             VALUES (?1, 1, 0, ?2)
             ON CONFLICT(symbol_id) DO UPDATE SET used_count = used_count + 1, last_used = ?2",
        rusqlite::params![id, now],
    )?;
    Ok(())
}

/// Upsert the `irrelevant` side of the per-symbol counter row: insert with
/// `irrelevant_count = 1` or bump the existing count. `last_used` is left
/// untouched — a dismissal is not a use. Mirrors
/// [`crate::stats::feedback::upsert_irrelevant`].
pub(crate) fn upsert_code_irrelevant(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO code_feedback(symbol_id, used_count, irrelevant_count)
             VALUES (?1, 0, 1)
             ON CONFLICT(symbol_id) DO UPDATE SET irrelevant_count = irrelevant_count + 1",
        rusqlite::params![id],
    )?;
    Ok(())
}

/// Insert one code-tagged `feedback_events` provenance row, text-encoding
/// the symbol id into the `memory_id` column (see the module doc for the
/// column-name wart). Private helper so
/// [`record_code_with_provenance`]'s used and irrelevant loops share the
/// INSERT SQL.
fn insert_code_event(
    conn: &Connection,
    query_id: &str,
    id: i64,
    verdict: &str,
    at: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO feedback_events(query_id, memory_id, verdict, at, target_kind)
         VALUES (?1, ?2, ?3, ?4, 'code')",
        rusqlite::params![query_id, id.to_string(), verdict, at],
    )?;
    Ok(())
}

/// Record a batch of used/irrelevant code-symbol verdicts for one query in
/// a single transaction: one code-tagged `feedback_events` row per id plus
/// the matching `code_feedback` counter upsert, written together per id.
/// All-or-nothing — a failure on any id leaves both tables untouched, so
/// events and counters cannot drift. Mirrors
/// [`crate::stats::feedback::record_with_provenance`], including the
/// recorded-verbatim query-id contract and the
/// [`memory_row::iso_format`] timestamp shared with `retrieval_log.at`.
pub fn record_code_with_provenance(
    db: &mut StatsDb,
    query_id: &str,
    used: &[i64],
    irrelevant: &[i64],
) -> Result<()> {
    let now = memory_row::iso_format(OffsetDateTime::now_utc())?;
    let tx = db.conn_mut().transaction()?;
    for id in used {
        insert_code_event(&tx, query_id, *id, "used", &now)?;
        upsert_code_used(&tx, *id, &now)?;
    }
    for id in irrelevant {
        insert_code_event(&tx, query_id, *id, "irrelevant", &now)?;
        upsert_code_irrelevant(&tx, *id)?;
    }
    tx.commit()?;
    Ok(())
}
