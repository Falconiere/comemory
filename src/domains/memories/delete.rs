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
use crate::store::memory_intent::{self, Intent, IntentKind};
use crate::store::replica_journal::{ReplicaOp, ReplicaOrigin};
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
    let removed = soft_delete(paths, conn, id, Some(ReplicaOrigin::Local), None)?;
    Ok(Response {
        deleted: removed.id,
        derived_stale: removed.derived_stale,
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
    at: Option<&str>,
) -> Result<SoftDeleted> {
    let store = MemoryStore::new(paths.clone());
    // Resolved before the markdown moves, so the intent below names the
    // canonical id and the live path rather than the caller's argument. The
    // second load inside `delete` hits the warmed path cache.
    let found = store.load(id)?;
    let operation_id = journal::mint_operation_id(&found.frontmatter.id, ReplicaOp::Tombstone);
    if journal_as.is_some() {
        // A deletion that replicates owes an operation, so it owes an intent:
        // a process killed between the `.trash/` move and the transaction
        // below would otherwise leave a memory gone from disk whose deletion
        // no peer will ever hear about. A heal (`journal_as: None`) makes no
        // new deletion and owes nothing.
        memory_intent::record(
            conn,
            &Intent {
                entity_key: found.frontmatter.id.clone(),
                kind: IntentKind::Delete,
                md_path: found.path.to_string_lossy().into_owned(),
                operation_id: operation_id.clone(),
                started_at: memory_row::iso_format(OffsetDateTime::now_utc())?,
            },
        )?;
    }
    let removed = store.delete(id)?;
    let content_hash = removed.frontmatter.content_hash.clone();
    let repository = removed.frontmatter.repo.clone();
    let id = removed.frontmatter.id;
    let tombstone = journal_as.map(|origin| Tombstone {
        content_hash: content_hash.as_str(),
        repository: (!repository.is_empty()).then_some(repository.as_str()),
        origin,
        at,
        operation_id: operation_id.as_str(),
    });
    let (derived_stale, journalled) = mirror_soft_delete(conn, &id, tombstone)?;
    Ok(SoftDeleted {
        id,
        derived_stale,
        journalled,
    })
}

/// What one soft-delete did.
#[derive(Debug, Clone)]
pub(crate) struct SoftDeleted {
    /// Canonical id from the removed record's frontmatter.
    pub id: String,
    /// Whether the derived-artifact refresh that follows the mirror write
    /// failed, leaving the relation index stale.
    pub derived_stale: bool,
    /// Where the deletion landed in each feed, when it was journalled — in the
    /// same transaction as the mirror delete, so a caller never has to open a
    /// second one to record it.
    pub journalled: Option<journal::Positions>,
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
    /// Provenance time to record. `None` stamps the delete's own clock, which
    /// is right for a local deletion; an import passes the time the peer sent.
    pub at: Option<&'a str>,
    /// The operation id the write intent named, so the deletion that finishes
    /// — now, or at the next reconciliation — journals under one id.
    pub operation_id: &'a str,
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
) -> Result<(bool, Option<journal::Positions>)> {
    let now = memory_row::iso_format(OffsetDateTime::now_utc())?;
    let tx = conn.transaction()?;
    memory_purge::soft_delete(&tx, id, &now)?;
    let mut journalled = None;
    if let Some(tombstone) = tombstone {
        // In the delete's own transaction: a memory whose markdown is gone
        // but whose deletion was never journalled would come back on the next
        // pull.
        journalled = Some(journal::record_tombstone(
            &tx,
            id,
            tombstone.content_hash,
            tombstone.repository,
            tombstone.at.unwrap_or(now.as_str()),
            tombstone.origin,
            Some(tombstone.operation_id),
        )?);
        // Inside the delete's own transaction, for the same reason the
        // tombstone is: an aborted delete must still owe reconciliation.
        memory_intent::clear(&tx, id)?;
    }
    tx.commit()?;
    // After the commit, so a failed refresh cannot roll back a delete that
    // succeeded — and reported rather than swallowed, since a stale
    // relation index is something the caller can pass on.
    let derived_stale = !crate::domains::graph::derived::refresh_derived_best_effort(conn);
    Ok((derived_stale, journalled))
}
