//! `memories::restore` — `POST /api/v1/memories/{id}/restore` /
//! `POST /api/v1/trash/{id}/restore`: bring a soft-deleted memory back
//! (console-api spec §4/§9).
//!
//! The exact reverse of `delete::soft_delete`: that surface moves
//! `memories/{id}-{slug}.md` into `.trash/`, stamps `deleted_at`, and drops
//! the FTS/vector rows and every touching edge. Restore moves the file back
//! (`MemoryStore::restore`, which refuses to rename over a live re-save of
//! the same body), re-derives the INCOMING relation edges from the live
//! tree's frontmatter (they belong to other memories, so the restored
//! markdown cannot regenerate them), then re-runs
//! `store::memory_row::insert`, whose `MEMORIES_UPSERT_SQL` sets
//! `deleted_at = NULL` while rebuilding tags, `memory_fts`, the outgoing
//! edges and the `code_ref` anchors from the markdown — so the memory comes
//! back live, searchable and re-linked in both directions.
//!
//! One thing it cannot restore: the `memory_vec` row. Delete drops it and
//! only the caller's embedder can produce another (the BYO-vector contract),
//! so a restored memory is lexical-only until it is re-saved with a vector.

use std::time::Instant;

use serde::Serialize;
use time::OffsetDateTime;

use crate::domains::memories::journal;
use crate::domains::memories::{MemoryRecord, MemoryStore};
use crate::prelude::*;
use crate::store::edges::{self, EdgeKey};
use crate::store::memory_intent::{self, Intent, IntentKind};
use crate::store::replica_journal::{ReplicaOp, ReplicaOrigin};
use crate::store::{Connection, memory_row};
use crate::utilities::activity::{self, Outcome, command};
use crate::utilities::context::Ctx;

/// `POST /api/v1/memories/{id}/restore` / `POST /api/v1/trash/{id}/restore`
/// response.
#[derive(Serialize, Debug)]
pub struct Response {
    /// Canonical id of the restored memory.
    pub id: String,
    /// On-disk path the markdown file was restored to, back under
    /// `memories/`.
    pub path: String,
    /// The restore left the derived artifacts stale: `edge_fts` could not be
    /// rebuilt after the row and its incoming edges came back. The restore
    /// itself committed — a freshness warning, not a failure. Omitted when
    /// false, as in `gc`, `delete`, `prune` and `update`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub derived_stale: bool,
}

/// Restore one soft-deleted memory. `Error::NotFound` when no `.trash/`
/// file carries the id, `Error::BadRequest` when the id names a live
/// memory (there is nothing to restore).
///
/// The file move and the SQLite mirror are two steps, not one transaction:
/// a mirror failure leaves the markdown live under `memories/` with the row
/// still stamped `deleted_at`, so the error names the path and the
/// `comemory rebuild` recovery, exactly as `memories::save` does.
pub fn run(ctx: &mut Ctx<'_>, id: &str) -> Result<Response> {
    let started = Instant::now();
    let result = restore_one(ctx, id);
    let summary = result
        .as_ref()
        .map(|r| serde_json::json!({"id": r.id, "derived_stale": r.derived_stale}));
    let outcome = match &summary {
        Ok(value) => Outcome::Ok(value),
        Err(e) => Outcome::Failed(e),
    };
    activity::record_in(ctx, command::RESTORE, started, &outcome, None);
    result
}

/// The restore itself, wrapped by [`run`] so the activity row is written
/// once, outside the work it describes.
///
/// `pub(crate)` for one caller: `sync::exchange::import_rules` restores a
/// trashed memory as part of applying an import batch, and that batch already
/// records itself as one `sync.import` run. Going through [`run`] there would
/// write a second `restore` row per entry for work the batch has already
/// reported — the same trap `memories::update` avoids by calling
/// `save::run_with` instead of `save::run`.
pub(crate) fn restore_one(ctx: &mut Ctx<'_>, id: &str) -> Result<Response> {
    let store = MemoryStore::new(ctx.paths.clone());
    // Before the markdown leaves `.trash/`: a process killed between the move
    // back and the journal below would otherwise leave a memory live on disk
    // whose restore no peer will ever hear about.
    let trashed = store.trashed_record(id)?;
    let file_name = trashed.path.file_name().ok_or_else(|| {
        Error::Other(format!(
            "trashed memory path has no file name: {}",
            trashed.path.display()
        ))
    })?;
    let live_path = ctx.paths.memories_dir().join(file_name);
    let operation_id = journal::mint_operation_id(&trashed.frontmatter.id, ReplicaOp::Restore);
    let started_at = memory_row::iso_format(OffsetDateTime::now_utc())?;
    memory_intent::record(
        ctx.conn()?,
        &Intent {
            entity_key: trashed.frontmatter.id.clone(),
            kind: IntentKind::Write,
            md_path: live_path.to_string_lossy().into_owned(),
            operation_id: operation_id.clone(),
            started_at,
        },
    )?;
    let record = store.restore(id)?;
    let derived_stale = mirror(ctx, &store, &record).map_err(|e| {
        Error::Other(format!(
            "restore: markdown at {} is back under memories/ but the SQLite mirror failed: {}; \
             run `comemory rebuild` to reconcile",
            record.path.display(),
            e
        ))
    })?;
    append_local_restore(ctx.conn()?, &record, &operation_id)?;
    Ok(Response {
        id: record.frontmatter.id.clone(),
        path: record.path.to_string_lossy().into_owned(),
        derived_stale,
    })
}

/// The SQLite half of a restore: the incoming relation edges first (their
/// own transaction), then the row itself through the shared
/// `memories::update::mirror_record`, which also refreshes the derived artifacts
/// — once, after both halves are in place.
/// Returns whether that refresh failed, so `run` can report it.
fn mirror(ctx: &mut Ctx<'_>, store: &MemoryStore, record: &MemoryRecord) -> Result<bool> {
    let id = record.frontmatter.id.as_str();
    let live = store.list()?;
    let conn = ctx.conn()?;
    let tx = conn.transaction()?;
    let relinked = relink_incoming(&tx, &live, id)?;
    tx.commit()?;
    tracing::debug!(
        memory_id = id,
        relinked,
        "restore re-derived incoming relation edges"
    );
    crate::domains::memories::update::mirror_record(ctx, record)
}

/// Soft-delete removes every edge touching the memory, both directions. The
/// outgoing ones come back from its own frontmatter in `memory_row::insert`,
/// but an INCOMING relation edge (`B —supersedes→ A`) lives in *B's*
/// frontmatter, so re-derive those from the live tree: every live memory
/// naming `id` under `supersedes` / `conflicts_with` / `derived_from` gets
/// its edge toward `id` upserted. Rebuild-style fresh timestamps — the
/// originals were dropped with the edges. O(N) file reads, acceptable for an
/// admin action. Returns the number of edges emitted.
fn relink_incoming(conn: &Connection, live: &[MemoryRecord], id: &str) -> Result<usize> {
    let mut emitted = 0;
    for rec in live {
        let fm = &rec.frontmatter;
        if fm.id == id {
            continue;
        }
        for (rel, ids) in [
            ("supersedes", &fm.relations.supersedes),
            ("conflicts_with", &fm.relations.conflicts_with),
            ("derived_from", &fm.relations.derived_from),
        ] {
            if !ids.iter().any(|dst| dst == id) {
                continue;
            }
            edges::insert(
                conn,
                EdgeKey {
                    src_kind: "memory",
                    src_id: &fm.id,
                    dst_kind: "memory",
                    dst_id: id,
                    rel,
                },
            )?;
            emitted += 1;
        }
    }
    Ok(emitted)
}

/// Journal a restore after the mirror succeeds: the legacy `sync_log` row,
/// the replica feed position and the outbox row it owes, in one transaction.
fn append_local_restore(
    conn: &mut Connection,
    record: &MemoryRecord,
    operation_id: &str,
) -> Result<()> {
    let fm = &record.frontmatter;
    let at = memory_row::iso_format(fm.created)?;
    let tx = conn.transaction()?;
    journal::record_write(
        &tx,
        ReplicaOp::Restore,
        fm,
        &record.body,
        &at,
        ReplicaOrigin::Local,
        Some(operation_id),
    )?;
    // The intent clears in the LAST transaction the restore owes, not the
    // mirror one: a restore whose row landed but whose operation did not is
    // still unfinished.
    memory_intent::clear(&tx, &fm.id)?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/restore.rs"]
mod tests;
