//! The persistence half of [`super::save`] — write the markdown record,
//! then mirror it into `comemory.db` in one transaction, with the write
//! intent bracketing both.
//!
//! Split out of `save.rs` to keep each file inside the structure gate's line
//! ceiling; the ordering it encodes is the whole point and belongs in one
//! place: intent, markdown, then mirror + journal + clear.

use crate::domains::memories::journal;
use crate::domains::memories::{MemoryStore, SaveParams, id, mirror};
use crate::prelude::*;
use crate::store::memory_intent::{self, Intent, IntentKind};
use crate::store::replica_journal::{ReplicaOp, ReplicaOrigin};
use crate::store::{Connection, memory_row, vector};

/// Write the markdown record (source of truth), then mirror it into
/// `comemory.db` in one transaction. A mirror failure keeps the markdown and
/// names it plus the `rebuild` recovery path.
///
/// The write intent goes down BEFORE the markdown moves, and is cleared
/// inside the mirror's own transaction. A process killed in that window
/// leaves a memory on disk the database has never seen; the outstanding
/// intent is what lets the next open finish it (`memories::recover`).
pub(super) fn persist(
    conn: &mut Connection,
    store: &MemoryStore,
    params: SaveParams<'_>,
    vector_opt: Option<&[f32]>,
) -> Result<crate::domains::memories::MemoryRecord> {
    let tags = params.tags.to_vec();
    let entity_key = id::memory_id(params.body);
    let operation_id = journal::mint_operation_id(&entity_key, ReplicaOp::Upsert);
    memory_intent::record(
        conn,
        &Intent {
            entity_key,
            kind: IntentKind::Write,
            md_path: store.planned_path(params.body).to_string_lossy().into_owned(),
            operation_id: operation_id.clone(),
            started_at: memory_row::iso_format(time::OffsetDateTime::now_utc())?,
        },
    )?;
    let rec = store.save(params)?;
    let md_path = rec.path.clone();
    write_sqlite_mirror(conn, &rec, &tags, vector_opt, &operation_id).map_err(|e| {
        if crate::store::busy::is_locked(&e) {
            // Keep the retryable error class through CLI, HTTP and MCP.
            // Retrying this content-derived save safely repairs the mirror.
            return e;
        }
        Error::Other(format!(
            "save: markdown at {} was written but SQLite mirror failed: {}; \
             run `comemory rebuild` to reconcile",
            md_path.display(),
            e
        ))
    })?;
    let _stale = crate::domains::graph::derived::refresh_derived_best_effort(conn);
    Ok(rec)
}

/// Mirror the markdown record into `comemory.db` in a single transaction:
/// `memories`, `memory_tags`, `memory_fts`, optional `memory_vec`, and the
/// graph `edges` table. The non-vector branch is delegated to
/// [`memory_row::insert`] so save and `comemory rebuild` cannot drift.
fn write_sqlite_mirror(
    conn: &mut Connection,
    rec: &crate::domains::memories::MemoryRecord,
    tags: &[String],
    vector_opt: Option<&[f32]>,
    operation_id: &str,
) -> Result<()> {
    let tx = crate::store::connection::write_transaction(conn)?;
    let fm = &rec.frontmatter;
    let md_path = rec.path.to_string_lossy();
    mirror::insert_row(&tx, fm, &rec.body, rec.slug.as_str(), &md_path, tags)?;
    if let Some(v) = vector_opt {
        // A re-save of the same id must replace, not duplicate, its
        // memory_vec row (see `store::vector::replace_memory`'s doc).
        vector::replace_memory(&tx, &fm.id, v)?;
    }
    let at = memory_row::iso_format(fm.created)?;
    journal::record_write(
        &tx,
        ReplicaOp::Upsert,
        fm,
        &rec.body,
        &at,
        ReplicaOrigin::Local,
        Some(operation_id),
    )?;
    // Inside this transaction, never after it: clearing in a later one would
    // reopen the same crash window a statement wide.
    memory_intent::clear(&tx, &fm.id)?;
    tx.commit()?;
    Ok(())
}

