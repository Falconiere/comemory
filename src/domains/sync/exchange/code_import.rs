//! `POST /sync/code/import` — apply one batch of a repo's snippet-free code
//! projection: replace files, drop removed ones, optionally replace the
//! co-change set, stamp the marker, and re-rank.
//!
//! One transaction per batch. A batch that fails validation writes nothing
//! and reports every rejection, so `repo_marker.last_head` only ever names
//! a head whose files all landed (AC-7). A file whose `blob_oid` already
//! matches the stored row is a no-op counted as applied (AC-5); a changed
//! one has its symbols and outgoing `imports` edges replaced whole, never
//! merged (AC-6). Rank is not on the wire: the tail runs the same
//! `materialize::recompute_rank` an explicit `POST /graph/recompute` does
//! (AC-10). Nothing here touches a memory row or `sync_log`.

use crate::domains::graph::{derived, materialize};
use crate::domains::sync::exchange::code_import_rules::{MAX_FILES, validate};
use crate::domains::sync::exchange::code_import_write::{remove_file, write_file};
use crate::domains::sync::exchange::code_types::{CodeImportRequest, CodeImportResponse};
use crate::prelude::*;
use crate::store::edges::{self, EdgeKey, file_node_id, file_node_prefix};
use crate::store::{Transaction, code_row, code_sync, repo_marker};
use crate::utilities::context::Ctx;

/// Apply `req` or refuse it whole.
///
/// # Errors
/// Store failures. Validation failures are not errors: they come back as
/// `rejected` with nothing written.
pub fn run(ctx: &mut Ctx<'_>, req: CodeImportRequest) -> Result<CodeImportResponse> {
    let conn = ctx.conn()?;
    // The quota is judged on the post-batch total: rows on the files this
    // batch replaces are about to go, so they must not count twice. A batch
    // over the file cap is refused by `validate` regardless, so it gets no
    // store round trip at all.
    let existing = if req.files.len() > MAX_FILES {
        0
    } else {
        let replaced: Vec<&str> = req.files.iter().map(|f| f.path.as_str()).collect();
        code_sync::symbol_count_excluding(conn, &req.repo, &replaced)?
    };
    let rejected = validate(&req, existing);
    if !rejected.is_empty() {
        return Ok(CodeImportResponse {
            applied: 0,
            removed: 0,
            rejected,
            head: repo_marker::last_head(conn, &req.repo)?,
        });
    }
    let tx = conn.transaction()?;
    let (applied, removed) = apply(&tx, &req)?;
    let head = repo_marker::last_head(&tx, &req.repo)?;
    tx.commit()?;
    let _stale = derived::refresh_derived_best_effort(conn);
    Ok(CodeImportResponse {
        applied,
        removed,
        rejected: Vec::new(),
        head,
    })
}

/// The batch inside its transaction: files, removals, co-change, marker,
/// rank. Returns `(applied, removed)`.
fn apply(tx: &Transaction<'_>, req: &CodeImportRequest) -> Result<(usize, usize)> {
    // The format stamp is what keeps a later `index-code` on this data dir
    // from wiping the cursors an import wrote — every `indexed_files` writer
    // must stamp, or `ensure_repo_format` treats the rows as pre-format.
    code_row::stamp_repo_format(tx, &req.repo)?;
    let mut applied = 0usize;
    for file in &req.files {
        write_file(tx, &req.repo, file)?;
        applied += 1;
    }
    let mut removed = 0usize;
    for path in &req.removed {
        remove_file(tx, &req.repo, path)?;
        removed += 1;
    }
    if let (Some(pairs), Some(cursor)) = (&req.cochange, &req.mined_commit) {
        replace_cochange(tx, &req.repo, pairs)?;
        repo_marker::advance_mined_cursor(tx, &req.repo, cursor)?;
    }
    if let Some(head) = &req.head {
        code_row::upsert_last_indexed(tx, &req.repo, head)?;
    }
    materialize::recompute_rank(tx, &req.repo)?;
    Ok((applied, removed))
}

/// Replace the repo's `co_changed` set wholesale: the pushing side sends the
/// whole set keyed by its mining cursor, so adding onto stored weights would
/// double-count every pair that survived.
fn replace_cochange(
    tx: &Transaction<'_>,
    repo: &str,
    pairs: &[crate::domains::sync::exchange::code_types::CoChangeWire],
) -> Result<()> {
    edges::delete_co_changed_for_repo(tx, &file_node_prefix(repo))?;
    for pair in pairs {
        // Canonical `a < b` order, as `graph::cochange` stores it.
        let (a, b) = if pair.from <= pair.to {
            (&pair.from, &pair.to)
        } else {
            (&pair.to, &pair.from)
        };
        let src = file_node_id(repo, a);
        let dst = file_node_id(repo, b);
        edges::insert_weighted(
            tx,
            EdgeKey {
                src_kind: "file",
                src_id: &src,
                dst_kind: "file",
                dst_id: &dst,
                rel: "co_changed",
            },
            pair.weight,
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/code_import.rs"]
mod tests;
