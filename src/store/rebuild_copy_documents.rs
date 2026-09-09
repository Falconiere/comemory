//! The document-domain third of [`super::rebuild_copy`]'s preservation
//! copy: `source_files`, `documents`, `document_chunks`, `document_fts`.
//!
//! `source_roots` — the fifth v13 table — is deliberately absent from this
//! module: it is reconstructed, not copied, from `sources.toml`
//! (`source::mirror::reconcile`, run by `api::rebuild::build_new_db` before
//! this copy runs). That ordering is load-bearing: `source_files.source_id`
//! is a `REFERENCES source_roots(id)` foreign key and the connection runs
//! with `PRAGMA foreign_keys=ON`, so an `INSERT OR IGNORE` into
//! `main.source_files` whose parent row is missing fails outright rather
//! than silently skipping the row — SQLite's `IGNORE` conflict resolution
//! does not cover foreign-key violations.

use crate::prelude::*;
use crate::store::Connection;
use crate::store::rebuild_copy::copy_table;

/// Copy `source_files`, `documents`, `document_chunks`, and `document_fts`
/// from `old` into `main`, in FK/parent-before-child order: `source_files`
/// before `documents` (`documents.source_file_id` references it),
/// `documents` before `document_chunks` (`document_chunks.document_id`
/// references it), and the FK-less `document_fts` virtual table last.
/// None of the four tables predates v13 (the current schema head), so no
/// column probing is needed, only the whole-table existence check.
pub(crate) fn copy_document_tables_inner(conn: &Connection) -> Result<()> {
    copy_source_files(conn)?;
    copy_documents(conn)?;
    copy_document_chunks(conn)?;
    copy_document_fts(conn)
}

/// Copy `source_files` rows: the discovery-walk candidate list (path,
/// classification, fingerprint, status) for each registered source.
fn copy_source_files(conn: &Connection) -> Result<()> {
    copy_table(
        conn,
        "source_files",
        "id, source_id, relative_path, classification, size, mtime, sha256, status, error, created_at, updated_at",
    )
}

/// Copy `documents` rows: one logical document per indexed source file.
fn copy_documents(conn: &Connection) -> Result<()> {
    copy_table(
        conn,
        "documents",
        "id, source_file_id, title, repo, revision_hash, created_at, updated_at",
    )
}

/// Copy `document_chunks` rows verbatim: the table has no surrogate id —
/// `(document_id, ordinal)` is its primary key.
fn copy_document_chunks(conn: &Connection) -> Result<()> {
    copy_table(
        conn,
        "document_chunks",
        "document_id, ordinal, heading_path, char_start, char_end, line_start, line_end, simhash, text",
    )
}

/// Copy `document_fts` rows via named columns — not every FTS5 shape
/// supports `SELECT *` from an attached DB.
fn copy_document_fts(conn: &Connection) -> Result<()> {
    copy_table(
        conn,
        "document_fts",
        "document_id, ordinal, title, headings, passage, path_tokens",
    )
}

#[cfg(test)]
#[path = "tests/rebuild_copy_documents.rs"]
mod tests;
