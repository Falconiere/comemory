//! `purge_memory` — hard-delete one **soft-deleted** memory's mirror rows
//! from `comemory.db`, in one transaction. The markdown half of the same
//! operation (unlinking `memories/.trash/{id}-{slug}.md`) is
//! `api::gc`'s trash sweep; this is the row half `gc` runs for every file
//! it reaps, and for the zombie rows earlier `gc` runs left behind
//! ([`expired_deleted_ids`]) — before this module existed a reaped memory
//! kept its `memories` row forever, listed in `GET /api/v1/trash` with no
//! file and counted by `stats.trashed` until a `comemory rebuild`.
//!
//! What goes: the `memories` row, its `memory_tags`, `memory_fts` and
//! `memory_vec` rows, every `edges` row touching it on either side (a
//! later `save --supersedes` pointing at a trashed memory is a real case),
//! its `code_ref` anchors, and the learning rows keyed by its id — the
//! `feedback` counter row and its memory-target `feedback_events`. None of
//! them can be reached once the memory is gone, and a memory id is only
//! ever reused by a byte-identical re-save, which should not inherit the
//! verdicts of a memory somebody deliberately deleted.
//!
//! What stays: `retrieval_log` (a row is one *query*; `returned_ids` is a
//! JSON list, not a key, and the row still describes its other ids), the
//! mined `query_expansions` (term → expansion, no memory key), and the run
//! histories (`eval_runs`, `gc_runs`, `index_runs`). Those are the
//! aggregated learning tables `comemory gc`'s retention window also leaves
//! alone, and nothing in them dangles on a purge.
//!
//! **Never a live row.** The `memories` delete carries `deleted_at IS NOT
//! NULL`; when it matches nothing the transaction is dropped unwritten and
//! the call reports `false`, so a caller that wrongly derives an id from a
//! filename cannot take a live memory with it.

use rusqlite::{Connection, params};

use crate::graph::edges;
use crate::prelude::*;

/// The per-memory tables keyed by a bare memory id, each cleared with the
/// id bound as `?1` once the guarded `memories` delete has matched.
/// `memory_tags` also cascades from the `memories` delete under
/// `PRAGMA foreign_keys=ON`; the explicit row keeps the purge complete on a
/// connection where that pragma is off.
const DEPENDENT_DELETES: &[&str] = &[
    "DELETE FROM memory_tags WHERE memory_id = ?1",
    "DELETE FROM memory_fts WHERE memory_id = ?1",
    "DELETE FROM memory_vec WHERE memory_id = ?1",
    "DELETE FROM code_ref WHERE memory_id = ?1",
    "DELETE FROM feedback WHERE memory_id = ?1",
];

/// Hard-delete every mirror row of the soft-deleted memory `id` in one
/// transaction (see the module doc for the table list). Returns `true`
/// when a soft-deleted row was found and purged, `false` — with nothing
/// written — when `id` is unknown or names a **live** memory.
pub fn purge_memory(conn: &mut Connection, id: &str) -> Result<bool> {
    let tx = conn.transaction()?;
    let matched = tx.execute(
        "DELETE FROM memories WHERE id = ?1 AND deleted_at IS NOT NULL",
        [id],
    )?;
    if matched == 0 {
        // Dropping `tx` without a commit rolls it back: nothing was written.
        return Ok(false);
    }
    for sql in DEPENDENT_DELETES {
        tx.execute(sql, [id])?;
    }
    edges::delete_touching(&tx, "memory", id)?;
    // `feedback_events.memory_id` also carries text-encoded code-symbol
    // rowids under `target_kind = 'code'`; an 8-digit rowid is a valid
    // memory-id shape, so the kind filter is what keeps code telemetry out.
    tx.execute(
        "DELETE FROM feedback_events WHERE memory_id = ?1 AND target_kind = ?2",
        params![id, crate::stats::target::MEMORY],
    )?;
    tx.commit()?;
    Ok(true)
}

/// Ids of the soft-deleted memories whose `deleted_at` is older than
/// `retention_days` — the rows `gc` may purge even when no trash file is
/// left to reap. Both sides go through `datetime()` so the stored ISO-8601
/// precision cannot invert the comparison. Ordered by id so a sweep is
/// deterministic.
pub fn expired_deleted_ids(conn: &Connection, retention_days: u32) -> Result<Vec<String>> {
    let modifier = format!("-{retention_days} days");
    let mut stmt = conn.prepare(
        "SELECT id FROM memories \
          WHERE deleted_at IS NOT NULL \
            AND datetime(deleted_at) < datetime('now', ?1) \
          ORDER BY id",
    )?;
    let ids = stmt
        .query_map([modifier], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(ids)
}

#[cfg(test)]
#[path = "tests/memory_purge.rs"]
mod tests;
