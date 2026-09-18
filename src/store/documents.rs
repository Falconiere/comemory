//! `documents` + `document_chunks` row CRUD — the two plain SQLite
//! tables the index writer ([`crate::domains::documents::document::writer`]) replaces
//! wholesale inside its per-file transaction. Mirrors the
//! `code_symbols`/`code_fts` split: this module owns the plain rows,
//! [`crate::store::document_fts`] owns the FTS5 virtual table.

use rusqlite::{Connection, params};

use super::{
    orm,
    schema_documents::{
        DocumentChunks, Documents, document_chunks as chunk, documents as col, source_files as file,
    },
};
use crate::prelude::*;
use toolu_orm::core::query_column::CommonOps;

/// Caller-supplied fields for [`upsert_document`].
pub struct DocumentUpsert<'a> {
    /// 32-hex-char document id (see
    /// [`crate::domains::documents::document::fingerprint::document_id_of`]).
    pub id: &'a str,
    /// Owning `source_files.id`.
    pub source_file_id: &'a str,
    /// First heading, else the file stem.
    pub title: &'a str,
    /// Optional repository label.
    pub repo: Option<&'a str>,
    /// Full 64-hex-char SHA-256 of the file's content.
    pub revision_hash: &'a str,
    /// RFC3339/ISO8601 timestamp used only on first insert.
    pub created_at: &'a str,
    /// RFC3339/ISO8601 timestamp, always refreshed.
    pub updated_at: &'a str,
}

/// One `documents` row as read back from SQLite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentRow {
    /// 32-hex-char document id.
    pub id: String,
    /// Owning `source_files.id`.
    pub source_file_id: String,
    /// First heading, else the file stem.
    pub title: String,
    /// Optional repository label.
    pub repo: Option<String>,
    /// Full 64-hex-char SHA-256 of the file's content.
    pub revision_hash: String,
    /// Row creation timestamp.
    pub created_at: String,
    /// Row last-update timestamp.
    pub updated_at: String,
}

/// Insert a fresh `documents` row, or refresh an existing one's mutable
/// fields (`title`/`repo`/`revision_hash`/`updated_at`), leaving
/// `created_at` untouched.
pub fn upsert_document(conn: &Connection, row: DocumentUpsert<'_>) -> Result<()> {
    conn.execute(
        "INSERT INTO documents(id, source_file_id, title, repo, revision_hash, created_at, updated_at) \
         VALUES(?1,?2,?3,?4,?5,?6,?7) \
         ON CONFLICT(id) DO UPDATE SET \
             title = excluded.title, repo = excluded.repo, \
             revision_hash = excluded.revision_hash, updated_at = excluded.updated_at",
        params![
            row.id,
            row.source_file_id,
            row.title,
            row.repo,
            row.revision_hash,
            row.created_at,
            row.updated_at,
        ],
    )?;
    Ok(())
}

/// Fetch one `documents` row by id, or `None` if it does not exist.
pub fn get_document(conn: &Connection, id: &str) -> Result<Option<DocumentRow>> {
    orm::query_optional(
        conn,
        Documents::select()
            .columns_typed(&[
                &col::id,
                &col::source_file_id,
                &col::title,
                &col::repo,
                &col::revision_hash,
                &col::created_at,
                &col::updated_at,
            ])
            .filter(col::id.eq(id))
            .to_sql(),
        document_row_from_sql,
    )
}

/// Delete the `documents` row for `id`. Cascades to `document_chunks`
/// via `ON DELETE CASCADE`; the caller is still responsible for the
/// FK-less `document_fts` rows (see
/// [`crate::store::document_fts::delete_document`]). A no-op (0 rows
/// affected) when `id` has no row — safe to call for a `source_files`
/// row that was never classified as a document.
pub fn delete_document(conn: &Connection, id: &str) -> Result<()> {
    orm::execute(conn, Documents::delete().filter(col::id.eq(id)).to_sql())?;
    Ok(())
}

/// One `document_chunks` row's fields, as replaced by [`replace_chunks`].
pub struct ChunkRow<'a> {
    /// 0-based position within the document's chunk list.
    pub ordinal: i64,
    /// `" > "`-joined heading breadcrumb; empty above the first heading
    /// or for a format with none.
    pub heading_path: &'a str,
    /// `(start, end)` Unicode-char range into the extractor's
    /// normalized text, `end` exclusive.
    pub char_range: (i64, i64),
    /// `(start, end)` inclusive 1-based line range.
    pub line_range: (i64, i64),
    /// 64-bit SimHash bit pattern, stored as the `i64` column value.
    pub simhash: i64,
    /// The chunk's raw text.
    pub text: &'a str,
}

/// Replace every `document_chunks` row for `document_id`: a wholesale
/// `DELETE` then one `INSERT` per chunk, matching the writer's
/// per-file "delete+reinsert" contract (no diffing — the corpus is
/// personal-scale).
pub fn replace_chunks(conn: &Connection, document_id: &str, chunks: &[ChunkRow<'_>]) -> Result<()> {
    orm::execute(
        conn,
        DocumentChunks::delete()
            .filter(chunk::document_id.eq(document_id))
            .to_sql(),
    )?;
    for c in chunks {
        orm::execute(
            conn,
            DocumentChunks::insert()
                .set(&chunk::document_id, document_id)
                .set(&chunk::ordinal, c.ordinal)
                .set(&chunk::heading_path, c.heading_path)
                .set(&chunk::char_start, c.char_range.0)
                .set(&chunk::char_end, c.char_range.1)
                .set(&chunk::line_start, c.line_range.0)
                .set(&chunk::line_end, c.line_range.1)
                .set(&chunk::simhash, c.simhash)
                .set(&chunk::text, c.text)
                .to_sql(),
        )?;
    }
    Ok(())
}

/// One `document_chunks` row's citation fields, as read back by
/// [`get_chunk`] — the provenance [`crate::domains::retrieval::doc_route`]
/// attaches to a document hit's best-chunk citation.
pub struct ChunkCitation {
    /// `" > "`-joined heading breadcrumb of the chunk.
    pub heading_path: String,
    /// `(start, end)` inclusive 1-based line range of the chunk.
    pub line_range: (i64, i64),
    /// The chunk's raw passage text.
    pub text: String,
}

/// Fetch one `document_chunks` row's citation fields by
/// `(document_id, ordinal)`, or `None` if it no longer exists (e.g. the
/// chunk was replaced by a re-index between the FTS match and this read).
pub fn get_chunk(
    conn: &Connection,
    document_id: &str,
    ordinal: i64,
) -> Result<Option<ChunkCitation>> {
    orm::query_optional(
        conn,
        DocumentChunks::select()
            .columns_typed(&[
                &chunk::heading_path,
                &chunk::line_start,
                &chunk::line_end,
                &chunk::text,
            ])
            .filter(chunk::document_id.eq(document_id))
            .filter(chunk::ordinal.eq(ordinal))
            .to_sql(),
        |r| {
            Ok(ChunkCitation {
                heading_path: r.get(0)?,
                line_range: (r.get(1)?, r.get(2)?),
                text: r.get(3)?,
            })
        },
    )
}

/// Fetch the source-relative path of the document at `id` — the join
/// target `search --path` glob filtering needs, since a `documents` row
/// carries no path of its own (it lives on the owning `source_files`
/// row). `None` when the document, or the `source_files` row it points
/// at, no longer exists.
pub fn get_document_path(conn: &Connection, id: &str) -> Result<Option<String>> {
    orm::query_optional(
        conn,
        Documents::select()
            .column_expr(&file::relative_path.qualified(), "relative_path")
            .join(file::id.table, file::id.equals(&col::source_file_id))
            .filter(col::id.eq(id))
            .to_sql(),
        |r| r.get(0),
    )
}

/// List every `documents.id` owned (via `source_files`) by `source_id` —
/// used by `comemory unindex` to explicitly purge `document_fts` rows
/// before deleting the owning `source_roots` row: the FK cascade removes
/// `source_files`/`documents`/`document_chunks`, but `document_fts` is a
/// virtual table with no FK, so its rows need this id list to clean up
/// independently.
pub fn document_ids_for_source(conn: &Connection, source_id: &str) -> Result<Vec<String>> {
    orm::query_all(
        conn,
        select_document_ids()
            .filter(file::source_id.eq(source_id))
            .to_sql(),
        |r| r.get(0),
    )
}

/// Look up a document by relative path within one source — `source_files`
/// carries `UNIQUE (source_id, relative_path)`, so this can never be
/// ambiguous. See [`crate::domains::graph::doc_link::derive_after_document`].
pub(crate) fn document_id_in_source(
    conn: &Connection,
    source_id: &str,
    relative_path: &str,
) -> Result<Option<String>> {
    orm::query_optional(
        conn,
        select_document_ids()
            .filter(file::source_id.eq(source_id))
            .filter(file::relative_path.eq(relative_path))
            .to_sql(),
        |r| r.get(0),
    )
}

/// Every live document id at `(repo, relative_path)` across every indexed
/// source — 0, 1 (resolves), or 2+ (ambiguous, caller's concern). See
/// [`crate::domains::graph::doc_link::derive_after_document`].
pub(crate) fn document_ids_for_repo_path(
    conn: &Connection,
    repo: &str,
    relative_path: &str,
) -> Result<Vec<String>> {
    orm::query_all(
        conn,
        select_document_ids()
            .filter(col::repo.eq(repo))
            .filter(file::relative_path.eq(relative_path))
            .to_sql(),
        |r| r.get(0),
    )
}

fn select_document_ids() -> toolu_orm::query::select::SelectBuilder {
    Documents::select()
        .column_expr(&col::id.qualified(), "id")
        .join(file::id.table, file::id.equals(&col::source_file_id))
}

fn document_row_from_sql(r: &rusqlite::Row<'_>) -> rusqlite::Result<DocumentRow> {
    Ok(DocumentRow {
        id: r.get(0)?,
        source_file_id: r.get(1)?,
        title: r.get(2)?,
        repo: r.get(3)?,
        revision_hash: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
    })
}

#[cfg(test)]
#[path = "tests/documents.rs"]
mod tests;
