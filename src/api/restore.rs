//! `api::restore` — `POST /api/v1/memories/{id}/restore` /
//! `POST /api/v1/trash/{id}/restore`: bring a soft-deleted memory back
//! (console-api spec §4/§9).
//!
//! The exact reverse of `cli::delete::soft_delete`: that surface moves
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

use rusqlite::Connection;
use serde::Serialize;

use crate::api::Ctx;
use crate::graph::edges::{self, EdgeKey};
use crate::memory::{MemoryRecord, MemoryStore};
use crate::prelude::*;

/// `POST /api/v1/memories/{id}/restore` / `POST /api/v1/trash/{id}/restore`
/// response.
#[derive(Serialize, Debug)]
pub struct Response {
    /// Canonical id of the restored memory.
    pub id: String,
    /// On-disk path the markdown file was restored to, back under
    /// `memories/`.
    pub path: String,
}

/// Restore one soft-deleted memory. `Error::NotFound` when no `.trash/`
/// file carries the id, `Error::BadRequest` when the id names a live
/// memory (there is nothing to restore).
///
/// The file move and the SQLite mirror are two steps, not one transaction:
/// a mirror failure leaves the markdown live under `memories/` with the row
/// still stamped `deleted_at`, so the error names the path and the
/// `comemory rebuild` recovery, exactly as `api::save` does.
pub fn run(ctx: &mut Ctx<'_>, id: &str) -> Result<Response> {
    let store = MemoryStore::new(ctx.paths.clone());
    let record = store.restore(id)?;
    mirror(ctx, &store, &record).map_err(|e| {
        Error::Other(format!(
            "restore: markdown at {} is back under memories/ but the SQLite mirror failed: {}; \
             run `comemory rebuild` to reconcile",
            record.path.display(),
            e
        ))
    })?;
    Ok(Response {
        id: record.frontmatter.id.clone(),
        path: record.path.to_string_lossy().into_owned(),
    })
}

/// The SQLite half of a restore: the incoming relation edges first (their
/// own transaction), then the row itself through the shared
/// `api::update::mirror_record`, which also refreshes the derived artifacts
/// — once, after both halves are in place.
fn mirror(ctx: &mut Ctx<'_>, store: &MemoryStore, record: &MemoryRecord) -> Result<()> {
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
    crate::api::update::mirror_record(ctx, record)
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

#[cfg(test)]
#[path = "tests/restore.rs"]
mod tests;
