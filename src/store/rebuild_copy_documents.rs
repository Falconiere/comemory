//! The document-domain third of [`super::rebuild_copy`]'s preservation
//! copy: `source_files`, `documents`, `document_chunks`, `document_fts`.
//!
//! `source_roots` — the fifth v13 table — is deliberately absent from this
//! module: it is reconstructed, not copied, from `sources.toml`
//! (`source::mirror::reconcile`, run by `maintenance::rebuild::build_new_db` before
//! this copy runs). That ordering is load-bearing: `source_files.source_id`
//! is a `REFERENCES source_roots(id)` foreign key and the connection runs
//! with `PRAGMA foreign_keys=ON`, so an `INSERT OR IGNORE` into
//! `main.source_files` whose parent row is missing fails outright rather
//! than silently skipping the row — SQLite's `IGNORE` conflict resolution
//! does not cover foreign-key violations.

use crate::prelude::*;
use crate::store::Connection;
use crate::store::rebuild_copy::{copy_table, old_table_exists};

/// `(table, columns)` for the four local document tables, in
/// FK/parent-before-child order: `source_files` before `documents`
/// (`documents.source_file_id` references it), `documents` before
/// `document_chunks` (`document_chunks.document_id` references it), and the
/// FK-less `document_fts` virtual table last. None of the four tables
/// predates v13 (the current schema head), so no column probing is needed,
/// only the whole-table existence check inside [`copy_table`].
const DOCUMENT_TABLES: &[(&str, &str)] = &[
    // The discovery-walk candidate list (path, classification, fingerprint,
    // status) for each registered source.
    (
        "source_files",
        "id, source_id, relative_path, classification, size, mtime, sha256, status, error, created_at, updated_at",
    ),
    // One logical document per indexed source file.
    (
        "documents",
        "id, source_file_id, title, repo, revision_hash, created_at, updated_at",
    ),
    // Verbatim chunks: no surrogate id — `(document_id, ordinal)` is the key.
    (
        "document_chunks",
        "document_id, ordinal, heading_path, char_start, char_end, line_start, line_end, simhash, text",
    ),
    ("document_vectors", "document_id, ordinal, model, embedding"),
    // Named columns — not every FTS5 shape supports `SELECT *` from an
    // attached DB.
    (
        "document_fts",
        "document_id, ordinal, title, headings, passage, path_tokens",
    ),
];

/// Copy every `(table, columns)` pair in `tables` through [`copy_table`].
/// The single shared looper for this module's const tables, so the four
/// table copies cannot drift into four near-identical helpers.
fn copy_all(conn: &Connection, tables: &'static [(&'static str, &'static str)]) -> Result<()> {
    for &(table, columns) in tables {
        copy_table(conn, table, columns)?;
    }
    Ok(())
}

/// Copy `source_files`, `documents`, `document_chunks`, and `document_fts`
/// from `old` into `main` ([`DOCUMENT_TABLES`] order), then the pulled
/// cache, approval map, and share mapping.
pub(crate) fn copy_document_tables_inner(conn: &Connection) -> Result<()> {
    copy_all(conn, DOCUMENT_TABLES)?;
    copy_shared_document_tables(conn)
}

/// `(table, columns)` for the tables #253 added, in FK order.
///
/// `document_share` comes after `documents` — `document_share.document_id`
/// references it, and with `PRAGMA foreign_keys=ON` an `INSERT OR IGNORE`
/// whose parent row is missing fails outright rather than skipping the row.
/// A share row whose document did not survive the copy is therefore dropped
/// by the same `IN (SELECT …)` narrowing the code side uses, not left to
/// abort the rebuild.
///
/// The four `remote_document*` tables have no foreign key and no local
/// dependency at all: a pulled revision describes a file this machine may not
/// have. They are copied because nothing on this disk could re-derive them.
///
/// `repository_approval` is server state for the same reason. Dropping it
/// would leave every document withheld until the next policy load, which a
/// rebuild gives an operator no sign of needing.
const SHARED_DOCUMENT_TABLES: &[(&str, &str)] = &[
    (
        "remote_document",
        "repo, shared_id, path, title, format, revision_hash, chunk_count, accepted_at",
    ),
    (
        "remote_document_chunk",
        "repo, shared_id, ordinal, heading_path, char_start, char_end, line_start,          line_end, simhash, text",
    ),
    ("remote_document_link", "repo, shared_id, ordinal, target"),
    (
        "remote_document_fts",
        "repo, shared_id, ordinal, title, headings, passage, path_tokens",
    ),
    ("repository_approval", "label, canonical, updated_at"),
];

/// Copy the pulled document cache, the approval map, and the share mapping
/// for every document that survived the copy above.
fn copy_shared_document_tables(conn: &Connection) -> Result<()> {
    copy_all(conn, SHARED_DOCUMENT_TABLES)?;
    if old_table_exists(conn, "document_share")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.document_share(                 document_id, repo, shared_id, path, blocked_reason, updated_at)              SELECT document_id, repo, shared_id, path, blocked_reason, updated_at              FROM old.document_share               WHERE document_id IN (SELECT id FROM main.documents);",
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/rebuild_copy_documents.rs"]
mod tests;
