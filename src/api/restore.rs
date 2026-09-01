//! `api::restore` — `POST /api/v1/memories/{id}/restore` /
//! `POST /api/v1/trash/{id}/restore`: bring a soft-deleted memory back
//! (console-api spec §4/§9).
//!
//! The exact reverse of `cli::delete::soft_delete`: that surface moves
//! `memories/{id}-{slug}.md` into `.trash/`, stamps `deleted_at`, and drops
//! the FTS/vector rows and every touching edge. Restore moves the file back
//! (`MemoryStore::restore`) and re-runs `store::memory_row::insert`, whose
//! `MEMORIES_UPSERT_SQL` sets `deleted_at = NULL` while rebuilding tags,
//! `memory_fts`, the outgoing edges and the `code_ref` anchors from the
//! markdown — so the memory comes back live, searchable and re-linked.
//!
//! One thing it cannot restore: the `memory_vec` row. Delete drops it and
//! only the caller's embedder can produce another (the BYO-vector contract),
//! so a restored memory is lexical-only until it is re-saved with a vector.

use serde::Serialize;

use crate::api::Ctx;
use crate::memory::MemoryStore;
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
pub fn run(ctx: &mut Ctx<'_>, id: &str) -> Result<Response> {
    let store = MemoryStore::new(ctx.paths.clone());
    let record = store.restore(id)?;
    crate::api::update::mirror_record(ctx, &record)?;
    Ok(Response {
        id: record.frontmatter.id.clone(),
        path: record.path.to_string_lossy().into_owned(),
    })
}

#[cfg(test)]
#[path = "tests/restore.rs"]
mod tests;
