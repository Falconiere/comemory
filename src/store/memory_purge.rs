//! `purge_memory` — hard-delete one **soft-deleted** memory's mirror rows
//! from `comemory.db`, in one transaction. The markdown half of the same
//! operation (unlinking `memories/.trash/{id}-{slug}.md`) is
//! `maintenance::gc`'s trash sweep; this is the row half `gc` runs for every file
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
//! Also erased: the journal copies of the shared verdicts on it (#254), so a
//! replay answers `payload_erased` instead of restoring them.
//!
//! What is REDACTED rather than kept or deleted: a `candidate_observations`
//! row naming this memory (#209). Unlike a `retrieval_log` row, it holds the
//! memory's body, so keeping it would resurrect content the user deleted;
//! unlike the tables above, deleting it would punch a hole in a recorded
//! candidate pool and corrupt the pool-recall figure that pool exists to
//! make measurable. Blanking the passage and marking the candidate
//! unresolved does neither.
//!
//! **Never a live row.** The `memories` delete carries `deleted_at IS NOT
//! NULL`; when it matches nothing the transaction is dropped unwritten and
//! the call reports `false`, so a caller that wrongly derives an id from a
//! filename cannot take a live memory with it.

use rusqlite::Connection;

use super::{
    orm,
    schema_graph::{CodeRef, code_ref},
    schema_learning::{Feedback, FeedbackEvents, feedback, feedback_events},
    schema_memory::{
        Memories, MemoryFts, MemoryTags, MemoryVec, memories, memory_fts, memory_tags, memory_vec,
    },
};
use crate::prelude::*;
use crate::store::edges;
use toolu_orm::core::query_column::CommonOps;

/// Mirror one soft-delete into `comemory.db`, inside the caller's already-open
/// transaction: stamp `deleted_at`, drop the `memory_fts` + `memory_vec`
/// rows (a `vec0` vtab has no FK cascade and no JOIN-side `deleted_at`
/// filter, so a surviving `memory_vec` row would block a future re-save of
/// the same body with a PK constraint failure), and delete every edge
/// touching the memory. Caller commits; see
/// [`crate::domains::memories::delete::mirror_soft_delete`] for why the
/// derived-artifact refresh runs after that commit rather than inside this
/// helper.
pub fn soft_delete(conn: &Connection, id: &str, now: &str) -> Result<()> {
    orm::execute(
        conn,
        Memories::update()
            .set(&memories::deleted_at, now)
            .filter(memories::id.eq(id))
            .to_sql(),
    )?;
    orm::execute(
        conn,
        MemoryFts::delete()
            .filter(memory_fts::memory_id.eq(id))
            .to_sql(),
    )?;
    orm::execute(
        conn,
        MemoryVec::delete()
            .filter(memory_vec::memory_id.eq(id))
            .to_sql(),
    )?;
    edges::delete_touching(conn, "memory", id)?;
    Ok(())
}

/// Hard-delete every mirror row of the soft-deleted memory `id` in one
/// transaction (see the module doc for the table list). Returns `true`
/// when a soft-deleted row was found and purged, `false` — with nothing
/// written — when `id` is unknown or names a **live** memory.
///
/// Takes the connection, not a `Transaction`, and opens its own: the
/// all-or-nothing purge IS the unit of work, and the caller (`maintenance::gc`)
/// loops over many ids, each independently durable. It must therefore be
/// called OUTSIDE an open transaction, which it checks rather than assumes:
/// a connection already in one is refused with a named error instead of the
/// opaque failure `rusqlite` would raise on the nested `BEGIN`.
pub fn purge_memory(conn: &mut Connection, id: &str) -> Result<bool> {
    if !conn.is_autocommit() {
        return Err(Error::Other(
            "purge_memory opens its own transaction and cannot run inside one".to_string(),
        ));
    }
    let tx = conn.transaction()?;
    let matched = orm::execute(
        &tx,
        Memories::delete()
            .filter(memories::id.eq(id))
            .filter(memories::deleted_at.is_not_null())
            .to_sql(),
    )?;
    if matched == 0 {
        // Dropping `tx` without a commit rolls it back: nothing was written.
        return Ok(false);
    }
    // Explicit cleanup also works when a caller disabled foreign-key cascades.
    for query in [
        MemoryTags::delete().filter(memory_tags::memory_id.eq(id)),
        MemoryFts::delete().filter(memory_fts::memory_id.eq(id)),
        MemoryVec::delete().filter(memory_vec::memory_id.eq(id)),
        CodeRef::delete().filter(code_ref::memory_id.eq(id)),
        Feedback::delete().filter(feedback::memory_id.eq(id)),
    ] {
        orm::execute(&tx, query.to_sql())?;
    }
    edges::delete_touching(&tx, "memory", id)?;
    // Redaction, not deletion — see the module doc. In the same transaction,
    // so a purge can never leave the body behind on a partial failure.
    super::candidate_observations::redact_memory(&tx, id)?;
    // A shared verdict on this memory leaves a journal copy that a replay
    // would otherwise restore (#254): erase those copies before the rows that
    // name their event ids are gone.
    let at = super::memory_row::iso_format(time::OffsetDateTime::now_utc())?;
    super::replica_redaction::redact(&tx, super::replica_redaction::Reach::VerdictsOn(id), &at)?;
    // `feedback_events.memory_id` also carries text-encoded code-symbol
    // rowids under `target_kind = 'code'`; an 8-digit rowid is a valid
    // memory-id shape, so the kind filter is what keeps code telemetry out.
    orm::execute(
        &tx,
        FeedbackEvents::delete()
            .filter(feedback_events::memory_id.eq(id))
            .filter(feedback_events::target_kind.eq(crate::utilities::telemetry::target::MEMORY))
            .to_sql(),
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

/// Whether a soft-deleted `memories` row carries `content_hash` — behind
/// `domains::sync::exchange::import_state`'s "already trashed under this hash" check.
pub fn trashed_with_hash(conn: &Connection, content_hash: &str) -> Result<bool> {
    let query = Memories::select()
        .filter(memories::content_hash.eq(content_hash))
        .filter(memories::deleted_at.is_not_null());
    orm::query_one(conn, query.to_exists_sql(), |r| r.get(0))
}

#[cfg(test)]
#[path = "tests/memory_purge.rs"]
mod tests;
