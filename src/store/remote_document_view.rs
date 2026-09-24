//! Reads over the pulled document cache that answer a question about the
//! corpus rather than about one document.
//!
//! [`super::remote_document`] reads one document by its shared id — what this
//! machine holds for it. This is the half of document SEARCH the pulled side
//! contributes, and the one place the precedence rule lives.

use rusqlite::Connection;

use crate::prelude::*;
use crate::store::remote_document::{self, Chunk};

/// What a peer has shared about one document this machine holds no file for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedDocument {
    /// Canonical repository.
    pub repo: String,
    /// 32 lowercase hex chars over `repo` and `path`.
    pub shared_id: String,
    /// Normalized repository-relative path.
    pub path: String,
    /// Title as the sender's extractor found it.
    pub title: String,
    /// The revision this machine holds.
    pub revision_hash: String,
}

/// The shared half of document search: every passage of an approved pulled
/// revision, minus any document the local index already holds.
///
/// Both rules live here so no caller can forget one. Approval is a subquery
/// rather than a later filter, so a revoked repository stops answering the
/// moment the map is replaced and answers again on reapproval without
/// re-indexing. The `NOT EXISTS` is precedence: a locally indexed document
/// answers from its own row, and the share mapping is the only thing the two
/// sides CAN be compared by — a local path is relative to its source root,
/// not to the repository. `ordinal` is cast because every fts5 column is text.
pub(crate) const SHARED_PASSAGES: &str = "SELECT \
         remote_document_fts.repo AS repo, \
         remote_document_fts.shared_id AS shared_id, \
         CAST(remote_document_fts.ordinal AS INTEGER) AS ordinal, \
         -bm25(remote_document_fts) AS score \
       FROM remote_document_fts \
      WHERE remote_document_fts MATCH ?1 \
        AND remote_document_fts.repo IN (SELECT canonical FROM repository_approval) \
        AND NOT EXISTS (SELECT 1 FROM document_share s \
                         WHERE s.repo = remote_document_fts.repo \
                           AND s.shared_id = remote_document_fts.shared_id)";

/// The pulled document one shared hit belongs to, or `None` when it is no
/// longer held (a tombstone between the match and this read).
///
/// # Errors
/// Propagates SQLite failures.
pub fn shared_document(
    conn: &Connection,
    repo: &str,
    shared_id: &str,
) -> Result<Option<SharedDocument>> {
    let mut statement = conn.prepare(
        "SELECT repo, shared_id, path, title, revision_hash FROM remote_document \
          WHERE repo = ?1 AND shared_id = ?2",
    )?;
    let mut rows = statement.query_map([repo, shared_id], |r| {
        Ok(SharedDocument {
            repo: r.get(0)?,
            shared_id: r.get(1)?,
            path: r.get(2)?,
            title: r.get(3)?,
            revision_hash: r.get(4)?,
        })
    })?;
    rows.next().transpose().map_err(Into::into)
}

/// One pulled passage, by `(repo, shared_id, ordinal)`.
///
/// Read through [`super::remote_document::all`] rather than a second query of
/// its own: a document's passages are bounded by that document, and one
/// definition of the chunk row is worth more than the rows not read.
///
/// # Errors
/// Propagates SQLite failures.
pub fn shared_passage(
    conn: &Connection,
    repo: &str,
    shared_id: &str,
    ordinal: i64,
) -> Result<Option<Chunk>> {
    Ok(remote_document::all::<Chunk>(conn, repo, shared_id)?
        .into_iter()
        .find(|chunk| chunk.ordinal == ordinal))
}

#[cfg(test)]
#[path = "tests/remote_document_view.rs"]
mod tests;
