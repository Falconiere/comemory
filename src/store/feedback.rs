//! `feedback` row CRUD: the per-memory `used`/`irrelevant` counter table,
//! plus the memory-tagged `feedback_events` provenance inserts.
//!
//! The provenance vocabulary (`stats::target`), the query-id contract, and
//! every transaction boundary stay in [`crate::stats::feedback`] — this
//! module owns only the SQL text and its parameter binding. See
//! [`crate::store::code_feedback`] for the code-side sibling table.

use rusqlite::{Connection, params};

use crate::prelude::*;

/// Upsert the `used` side of the per-memory counter row: insert with
/// `used_count = 1` or bump the existing count, refreshing `last_used` to
/// `now` either way. Accepts any [`Connection`] (a `rusqlite::Transaction`
/// derefs to one).
pub(crate) fn upsert_used(conn: &Connection, id: &str, now: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO feedback(memory_id, used_count, irrelevant_count, last_used)
             VALUES (?1, 1, 0, ?2)
             ON CONFLICT(memory_id) DO UPDATE SET used_count = used_count + 1, last_used = ?2",
        params![id, now],
    )?;
    Ok(())
}

/// Upsert the `irrelevant` side of the per-memory counter row: insert with
/// `irrelevant_count = 1` or bump the existing count. `last_used` is left
/// untouched — a dismissal is not a use.
pub(crate) fn upsert_irrelevant(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO feedback(memory_id, used_count, irrelevant_count)
             VALUES (?1, 0, 1)
             ON CONFLICT(memory_id) DO UPDATE SET irrelevant_count = irrelevant_count + 1",
        params![id],
    )?;
    Ok(())
}

/// Insert one memory-tagged `feedback_events` provenance row. `target_kind`
/// is the caller's `crate::stats::target` vocabulary constant, passed
/// explicitly rather than hardcoded so this helper stays table-shaped, not
/// domain-shaped.
pub(crate) fn insert_event(
    conn: &Connection,
    query_id: &str,
    id: &str,
    verdict: &str,
    at: &str,
    target_kind: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO feedback_events(query_id, memory_id, verdict, at, target_kind)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![query_id, id, verdict, at, target_kind],
    )?;
    Ok(())
}

/// Insert one implicit-`used` `feedback_events` row carrying an explicit
/// `provenance` tag (the co-activation reward / search→edit
/// reinforcement writers). Kept separate from [`insert_event`] because the
/// column list differs — an extra `provenance` column, `verdict` fixed to
/// `'used'` — mirroring the two distinct INSERT statements the pre-move
/// code ran.
pub(crate) fn insert_implicit_used_event(
    conn: &Connection,
    query_id: &str,
    id: &str,
    at: &str,
    target_kind: &str,
    provenance: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO feedback_events(query_id, memory_id, verdict, at, target_kind, provenance)
         VALUES (?1, ?2, 'used', ?3, ?4, ?5)",
        params![query_id, id, at, target_kind, provenance],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/feedback.rs"]
mod tests;
