//! Erase one document by shared id: a local document's derived rows and
//! `source_files` row (never the file itself), a pulled copy's cache, and the
//! bytes of every payload the entity's feed rows name. A tombstone is
//! journalled only for a document this engine shares; a pulled copy is
//! erased locally, since its tombstone would delete it for the workspace.

use super::{Applied, Ledger, clear_copies, commit, withdraw};
use crate::domains::documents::journal;
use crate::domains::documents::replica_payload::DOCUMENT_ENTITY_KIND;
use crate::prelude::*;
use crate::store::erase_rows::{self, FtsIndex};
use crate::store::replica_redaction::{self, Reach};
use crate::store::{Connection, replica_read};

/// Erase the document shared as `shared_id`. `NotFound`, with the
/// transaction rolled back, when no local share, pulled copy or journal
/// position names it.
pub(super) fn erase(
    ledger: Option<Ledger<'_>>,
    conn: &mut Connection,
    shared_id: &str,
    at: &str,
) -> Result<Applied> {
    // Read under the write lock, so an index racing the erase cannot slip a
    // row in between what was found and what is deleted.
    let tx = crate::store::connection::write_transaction(conn)?;
    let local = erase_rows::local_documents(&tx, shared_id)?;
    let journalled = replica_read::revision(&tx, DOCUMENT_ENTITY_KIND, shared_id)?.is_some();
    let mut tombstoned = false;
    for doc in &local {
        // Before the rows go: the tombstone reads the share row the
        // `documents` delete cascades away, and names what the revision did.
        tombstoned |= journal::record_tombstone(&tx, &doc.document_id, at)?.is_some();
        erase_rows::erase_local_document(&tx, doc)?;
    }
    let pulled = erase_rows::erase_pulled_document(&tx, shared_id)?;
    if local.is_empty() && !pulled && !journalled {
        // Dropping `tx` uncommitted rolls it back: nothing was written.
        return Err(Error::NotFound(format!("document {shared_id}")));
    }
    let operations_withdrawn = withdraw(&tx, DOCUMENT_ENTITY_KIND, shared_id, at)?;
    let digests =
        replica_redaction::redact(&tx, Reach::Entity(DOCUMENT_ENTITY_KIND, shared_id), at)?;
    let (replay_blanked, staged_removed) = clear_copies(&tx, &digests)?;
    commit(tx, ledger, (DOCUMENT_ENTITY_KIND, shared_id), &digests, at)?;
    let mut touched = Vec::new();
    if !local.is_empty() {
        touched.push(FtsIndex::Documents);
    }
    if pulled {
        touched.push(FtsIndex::PulledDocuments);
    }
    Ok(Applied {
        tombstoned,
        digests,
        operations_withdrawn,
        staged_removed,
        replay_blanked,
        touched,
    })
}
