//! The other half of the writer: what a discovery walk's ABSENCES mean.
//!
//! Split out of [`super::writer`] when that file crossed its size ceiling.
//! The seam is real rather than arbitrary — everything here runs once from a
//! completed walk and removes rows, where the writer runs per file and writes
//! them.
//!
//! The cases covering this module live in `tests/writer.rs`: each one
//! indexes a real document through the writer and then reconciles it away,
//! so the two halves are only meaningful together.

use std::collections::HashSet;

use super::fingerprint;
use super::writer::iso_now;
use crate::domains::documents::journal;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::document_fts;
use crate::store::documents;
use crate::store::edges;
use crate::store::sources::{self, SourceFileRow};

/// Given the FULL set of relative paths an authoritative discovery walk
/// of `source_id` just saw, tombstone every `source_files` row for that
/// source NOT in `seen`: its `documents`/`document_chunks`/
/// `document_fts` rows are removed and its own row flips to `deleted`
/// (spec step 5). A row already `deleted` is left alone — idempotent.
/// Returns the number of rows newly tombstoned.
///
/// # Errors
/// Propagates SQLite failures.
pub fn reconcile_deletions<S: ::std::hash::BuildHasher>(
    conn: &mut Connection,
    source_id: &str,
    seen: &HashSet<String, S>,
) -> Result<usize> {
    let mut removed = 0usize;
    for row in sources::list_files_by_source(conn, source_id)? {
        if row.status == "deleted" || seen.contains(&row.relative_path) {
            continue;
        }
        tombstone(conn, &row)?;
        removed += 1;
    }
    Ok(removed)
}

/// Remove `row`'s derived document rows (plus the `member_of_source` /
/// `references_document` edges it owns — `edges` has no FK, so a soft
/// delete must purge them explicitly) and flip it to `deleted`, all in
/// one transaction.
fn tombstone(conn: &mut Connection, row: &SourceFileRow) -> Result<()> {
    let document_id = fingerprint::document_id_of(&row.id);
    let now = iso_now()?;
    let tx = conn.transaction()?;
    // Before the delete: `document_share.document_id` cascades, so the row
    // naming what a peer received is gone the moment the parent is.
    journal::record_tombstone(&tx, &document_id, &now)?;
    documents::delete_document(&tx, &document_id)?;
    document_fts::delete_document(&tx, &document_id)?;
    edges::delete_touching(&tx, "file", &row.id)?;
    edges::delete_touching(&tx, "document", &document_id)?;
    sources::mark_deleted(&tx, &row.id, &now)?;
    tx.commit()?;
    Ok(())
}
