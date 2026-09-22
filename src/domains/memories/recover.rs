//! `memories::recover::reconcile` — finish, or drop, every memory write a
//! killed process left half-done.
//!
//! A memory write places its markdown and then mirrors it into `comemory.db`.
//! Between those two steps the file exists and the database has never seen it:
//! the memory is unfindable, and the upload it owes is recorded nowhere. The
//! [`crate::store::memory_intent`] row written before the markdown moves is
//! what makes that window recoverable, and this is the pass that closes it.
//!
//! Called from the three delivery seams (`cli::run`, `serve::serve`,
//! `mcp::serve`) rather than from `store::connection::open`: `store/` may not
//! call into a domain (`scripts/architecture-check.sh`, #177). It takes
//! `memory-save.lock` — the lock `memories::save` holds across markdown
//! staging and the mirror commit — because that is exactly the section it
//! re-enters, and a reconciliation racing a live save of the same id is the
//! one thing that could leave the pair inconsistent again.

use time::OffsetDateTime;

use crate::config::paths::Paths;
use crate::domains::memories::replica_payload::MEMORY_ENTITY_KIND;
use crate::domains::memories::{Frontmatter, journal, mirror};
use crate::prelude::*;
use crate::store::memory_intent::{Intent, IntentKind};
use crate::store::replica_journal::{ReplicaOp, ReplicaOrigin};
use crate::store::{
    Connection, memory_intent, memory_meta, memory_purge, memory_row, replica_read,
};
use crate::utilities::digest::sha256_hex;
use crate::utilities::file_lock::FileLock;

/// What one reconciliation pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    /// Writes completed: mirrored and journalled, or soft-deleted and
    /// journalled.
    pub finished: usize,
    /// Intents dropped because the markdown move they described never landed,
    /// so there is nothing to finish and nothing to undo.
    pub dropped: usize,
}

impl Report {
    /// Whether the pass had anything to do at all.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.finished == 0 && self.dropped == 0
    }
}

/// Finish every outstanding write intent against `conn`.
///
/// Idempotent: a second pass over a reconciled data directory finds no
/// intents and reports nothing. A read-only caller must not call this — see
/// [`reconcile_unless_read_only`].
///
/// # Errors
/// Propagates the lock acquisition, the markdown read and SQLite failures. A
/// failure leaves the remaining intents outstanding, so the next pass retries
/// them.
pub fn reconcile(paths: &Paths, conn: &mut Connection) -> Result<Report> {
    let outstanding = memory_intent::outstanding(conn)?;
    if outstanding.is_empty() {
        return Ok(Report::default());
    }
    let _guard = FileLock::acquire(&paths.data_dir().join("memory-save.lock"), "memory-save")?;
    let mut report = Report::default();
    // Re-read under the lock: a concurrent writer may have finished some of
    // them between the probe above and the lock being granted.
    for intent in memory_intent::outstanding(conn)? {
        if finish(conn, &intent)? {
            report.finished += 1;
        } else {
            report.dropped += 1;
        }
    }
    if report.finished > 0 {
        // After the commits, so a failed refresh cannot roll back writes that
        // succeeded. Best-effort: the next save or delete refreshes it again.
        let _stale = crate::domains::graph::derived::refresh_derived_best_effort(conn);
    }
    Ok(report)
}

/// [`reconcile`] unless `read_only`, in which case nothing is attempted.
///
/// A read-only session must not write, and an unfinished write is not lost by
/// waiting — the intent stays outstanding for the next writable open.
///
/// # Errors
/// Propagates [`reconcile`].
pub fn reconcile_unless_read_only(
    paths: &Paths,
    conn: &mut Connection,
    read_only: bool,
) -> Result<Report> {
    if read_only {
        return Ok(Report::default());
    }
    reconcile(paths, conn)
}

/// Finish one outstanding intent, or drop it.
///
/// Returns `false` when the markdown move the intent describes never landed:
/// a write whose file is not there has no memory to mirror and no operation
/// to journal, and a delete whose file is still live never deleted anything.
/// Both are dropped, leaving no memory, no operation and no orphan row.
fn finish(conn: &mut Connection, intent: &Intent) -> Result<bool> {
    match intent.kind {
        IntentKind::Write => {
            let Ok(raw) = std::fs::read_to_string(&intent.md_path) else {
                drop_intent(conn, &intent.entity_key)?;
                return Ok(false);
            };
            let (fm, body) = Frontmatter::split(&raw)?;
            let slug = crate::domains::memories::slug::slug_from_body(&body);
            let at = memory_row::iso_format(fm.created)?;
            commit_finish(
                conn,
                intent,
                // An upsert, so a write killed after its mirror commit but
                // before its journal commit costs one redundant row write
                // rather than a branch that could get the two cases the wrong
                // way round.
                |tx| mirror::insert_row(tx, &fm, &body, slug.as_str(), &intent.md_path, &fm.tags),
                |tx| {
                    journal::record_write(
                        tx,
                        op_of(tx, &intent.entity_key)?,
                        &fm,
                        &body,
                        &at,
                        ReplicaOrigin::Local,
                        Some(&intent.operation_id),
                    )
                    .map(|_| ())
                },
            )
        }
        IntentKind::Delete => {
            if std::path::Path::new(&intent.md_path).exists() {
                drop_intent(conn, &intent.entity_key)?;
                return Ok(false);
            }
            // The mirror row is the state being completed, so its own body is
            // what the legacy feed's content hash must describe — not the
            // trashed file, which gc may already have reaped. No row either
            // means the delete already committed; there is nothing to finish.
            let Some((_, body)) = memory_meta::kind_and_body(conn, &intent.entity_key)? else {
                drop_intent(conn, &intent.entity_key)?;
                return Ok(false);
            };
            let content_hash = sha256_hex(body.trim_end().as_bytes());
            let repository = memory_meta::fetch_meta(conn, &[intent.entity_key.as_str()])?
                .remove(&intent.entity_key)
                .and_then(|meta| meta.repo)
                .filter(|repo| !repo.is_empty());
            let now = memory_row::iso_format(OffsetDateTime::now_utc())?;
            commit_finish(
                conn,
                intent,
                |tx| memory_purge::soft_delete(tx, &intent.entity_key, &now),
                |tx| {
                    journal::record_tombstone(
                        tx,
                        &intent.entity_key,
                        &content_hash,
                        repository.as_deref(),
                        &now,
                        ReplicaOrigin::Local,
                        Some(&intent.operation_id),
                    )
                    .map(|_| ())
                },
            )
        }
    }
}

/// The one transaction every finished write commits: materialize the state,
/// journal the operation the intent named if it has no position yet, and
/// clear the intent — together or not at all.
///
/// `journal_op` runs only when the operation is genuinely missing from the
/// feed. A mirror row that landed proves nothing on its own, because the edit
/// and restore paths commit the mirror and the journal separately.
fn commit_finish<M, J>(
    conn: &mut Connection,
    intent: &Intent,
    materialize: M,
    journal_op: J,
) -> Result<bool>
where
    M: FnOnce(&Connection) -> Result<()>,
    J: FnOnce(&Connection) -> Result<()>,
{
    let tx = crate::store::connection::write_transaction(conn)?;
    materialize(&tx)?;
    if replica_read::position_of(&tx, &intent.operation_id)?.is_none() {
        journal_op(&tx)?;
    }
    memory_intent::clear(&tx, &intent.entity_key)?;
    tx.commit()?;
    Ok(true)
}

/// Forget an intent whose markdown move never landed.
fn drop_intent(conn: &mut Connection, entity_key: &str) -> Result<()> {
    let tx = crate::store::connection::write_transaction(conn)?;
    memory_intent::clear(&tx, entity_key)?;
    tx.commit()?;
    Ok(())
}

/// Which operation a finished write journals, read from the feed rather than
/// guessed from the intent.
///
/// A restore's intent is recorded while the memory is still tombstoned in the
/// journal, so a replica revision that says `deleted` is a restore being
/// finished; anything else is an upsert. The distinction matters to a peer: a
/// restore reverses a deletion it must name, an upsert does not.
fn op_of(tx: &Connection, entity_key: &str) -> Result<ReplicaOp> {
    let deleted = replica_read::revision(tx, MEMORY_ENTITY_KIND, entity_key)?
        .is_some_and(|revision| revision.deleted);
    Ok(if deleted {
        ReplicaOp::Restore
    } else {
        ReplicaOp::Upsert
    })
}

#[cfg(test)]
#[path = "tests/recover.rs"]
mod tests;
