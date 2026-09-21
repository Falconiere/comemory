//! `memories::delete::{Response, run}` — the shared middle of `comemory
//! delete` / `DELETE /api/v1/memories/{id}`, plus the two soft-delete helpers
//! every other deletion surface reuses. The confirm gate is transport-level
//! (`?confirm=true`), not part of this module — the CLI has no `--confirm`
//! concept, so `run` takes a plain `id`, not a `Request` struct.

use serde::Serialize;
use time::OffsetDateTime;

use std::time::Instant;

use crate::config::paths::Paths;
use crate::domains::memories::MemoryStore;
use crate::domains::memories::journal;
use crate::prelude::*;
use crate::store::replica_journal::ReplicaOrigin;
use crate::store::{Connection, memory_purge, memory_row};
use crate::utilities::activity::{self, Outcome, command};
use crate::utilities::context::Ctx;

/// `comemory delete` / `DELETE /api/v1/memories/{id}` response.
#[derive(Serialize, Debug)]
pub struct Response {
    /// Canonical id of the soft-deleted memory.
    pub deleted: String,
    /// The delete left the derived artifacts stale: `edge_fts` could not be
    /// rebuilt after the memory's edges went. The delete itself committed —
    /// this is a freshness warning, not a failure — but relation search is
    /// behind until the next write refreshes it. Omitted from the JSON when
    /// false, as in `gc`'s report.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub derived_stale: bool,
}

/// Soft-delete one memory: move the markdown file into `memories/.trash/`
/// and mirror the delete into `comemory.db`, via the shared `soft_delete`
/// helper — also reused by `comemory prune`'s low-value apply path and by the
/// sync import.
pub fn run(ctx: &mut Ctx<'_>, id: &str) -> Result<Response> {
    let started = Instant::now();
    let result = delete_one(ctx, id);
    let summary = result
        .as_ref()
        .map(|r| serde_json::json!({"id": r.deleted, "derived_stale": r.derived_stale}));
    let outcome = match &summary {
        Ok(value) => Outcome::Ok(value),
        Err(e) => Outcome::Failed(e),
    };
    activity::record_in(ctx, command::DELETE, started, &outcome, None);
    result
}

/// The soft-delete itself, wrapped by [`run`] so the activity row is written
/// once, outside the work it describes.
fn delete_one(ctx: &mut Ctx<'_>, id: &str) -> Result<Response> {
    let paths = ctx.paths;
    let conn = ctx.conn()?;
    let (deleted, _content_hash, derived_stale) =
        soft_delete(paths, conn, id, Some(ReplicaOrigin::Local))?;
    Ok(Response {
        deleted,
        derived_stale,
    })
}

/// Soft-delete one memory: move the markdown file into `memories/.trash/`
/// (source of truth), then mirror the delete into `comemory.db` via
/// [`mirror_soft_delete`]. Returns the canonical id from the removed
/// record's frontmatter.
///
/// Shared by `comemory delete` (via [`run`]) and `comemory prune` (low-value
/// apply path) so the two soft-delete surfaces cannot drift.
/// Returns the canonical id and whether the derived-artifact refresh that
/// follows the mirror write FAILED, so the caller can report a stale
/// relation index instead of leaving it in the log
/// ([`crate::domains::graph::derived::refresh_derived_best_effort`]).
pub(crate) fn soft_delete(
    paths: &Paths,
    conn: &mut Connection,
    id: &str,
    journal_as: Option<ReplicaOrigin>,
) -> Result<(String, String, bool)> {
    let removed = MemoryStore::new(paths.clone()).delete(id)?;
    let content_hash = removed.frontmatter.content_hash.clone();
    let repository = removed.frontmatter.repo.clone();
    let id = removed.frontmatter.id;
    let tombstone = journal_as.map(|origin| Tombstone {
        content_hash: content_hash.as_str(),
        repository: (!repository.is_empty()).then_some(repository.as_str()),
        origin,
    });
    let derived_stale = mirror_soft_delete(conn, &id, tombstone)?;
    Ok((id, content_hash, derived_stale))
}

/// What a soft-delete journals, for the surfaces that replicate their
/// deletions. `comemory prune`'s heal path passes `None`: it repairs a
/// half-deleted memory rather than making a new deletion.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Tombstone<'a> {
    /// Content hash the legacy `sync_log` row carries.
    pub content_hash: &'a str,
    /// Canonical repository, when the memory had one.
    pub repository: Option<&'a str>,
    /// Whether the deletion was made here or imported.
    pub origin: ReplicaOrigin,
}

/// Mirror a soft-delete into `comemory.db` in one transaction: stamp
/// `deleted_at`, drop the `memory_fts` + `memory_vec` rows, remove all
/// touching edges.
///
/// Factored out of [`soft_delete`] so `comemory prune` can heal a
/// half-deleted memory — live DB row, markdown already gone after a crash
/// between the file move and this transaction — with no markdown move.
///
/// After the commit the memory and its edges have left the graph, so
/// [`crate::domains::graph::derived`] refreshes both derived artifacts best-effort
/// here, not at the [`soft_delete`] call site: every soft-delete surface
/// (delete, prune apply, prune heal) then heals rank and triplets alike.
pub(crate) fn mirror_soft_delete(
    conn: &mut Connection,
    id: &str,
    tombstone: Option<Tombstone<'_>>,
) -> Result<bool> {
    let now = memory_row::iso_format(OffsetDateTime::now_utc())?;
    let tx = conn.transaction()?;
    memory_purge::soft_delete(&tx, id, &now)?;
    if let Some(tombstone) = tombstone {
        // In the delete's own transaction: a memory whose markdown is gone
        // but whose deletion was never journalled would come back on the next
        // pull.
        journal::record_tombstone(
            &tx,
            id,
            tombstone.content_hash,
            tombstone.repository,
            &now,
            tombstone.origin,
        )?;
    }
    tx.commit()?;
    // After the commit, so a failed refresh cannot roll back a delete that
    // succeeded — and reported rather than swallowed, since a stale
    // relation index is something the caller can pass on.
    Ok(!crate::domains::graph::derived::refresh_derived_best_effort(conn))
}
