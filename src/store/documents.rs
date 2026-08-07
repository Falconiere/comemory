//! `documents` + `document_chunks` row CRUD — the two plain SQLite
//! tables the index writer ([`crate::document::writer`]) replaces
//! wholesale inside its per-file transaction. Mirrors the
//! `code_symbols`/`code_fts` split: this module owns the plain rows,
//! [`crate::store::document_fts`] owns the FTS5 virtual table.

use rusqlite::{Connection, OptionalExtension, params};

use crate::prelude::*;

/// Caller-supplied fields for [`upsert_document`].
pub struct DocumentUpsert<'a> {
    /// 32-hex-char document id (see
    /// [`crate::document::writer::document_id_of`]).
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
    conn.query_row(
        "SELECT id, source_file_id, title, repo, revision_hash, created_at, updated_at \
           FROM documents WHERE id = ?1",
        params![id],
        document_row_from_sql,
    )
    .optional()
    .map_err(Error::from)
}

/// Delete the `documents` row for `id`. Cascades to `document_chunks`
/// via `ON DELETE CASCADE`; the caller is still responsible for the
/// FK-less `document_fts` rows (see
/// [`crate::store::document_fts::delete_document`]). A no-op (0 rows
/// affected) when `id` has no row — safe to call for a `source_files`
/// row that was never classified as a document.
pub fn delete_document(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM documents WHERE id = ?1", params![id])?;
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
    conn.execute(
        "DELETE FROM document_chunks WHERE document_id = ?1",
        params![document_id],
    )?;
    for c in chunks {
        conn.execute(
            "INSERT INTO document_chunks(\
                 document_id, ordinal, heading_path, char_start, char_end, \
                 line_start, line_end, simhash, text) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                document_id,
                c.ordinal,
                c.heading_path,
                c.char_range.0,
                c.char_range.1,
                c.line_range.0,
                c.line_range.1,
                c.simhash,
                c.text,
            ],
        )?;
    }
    Ok(())
}

/// One `document_chunks` row's citation fields, as read back by
/// [`get_chunk`] — the provenance [`crate::retrieval::doc_route`]
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
    conn.query_row(
        "SELECT heading_path, line_start, line_end, text FROM document_chunks \
          WHERE document_id = ?1 AND ordinal = ?2",
        params![document_id, ordinal],
        |r| {
            Ok(ChunkCitation {
                heading_path: r.get(0)?,
                line_range: (r.get(1)?, r.get(2)?),
                text: r.get(3)?,
            })
        },
    )
    .optional()
    .map_err(Error::from)
}

/// Fetch the source-relative path of the document at `id` — the join
/// target `search --path` glob filtering needs, since a `documents` row
/// carries no path of its own (it lives on the owning `source_files`
/// row). `None` when the document, or the `source_files` row it points
/// at, no longer exists.
pub fn get_document_path(conn: &Connection, id: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT sf.relative_path FROM documents d \
           JOIN source_files sf ON sf.id = d.source_file_id \
          WHERE d.id = ?1",
        params![id],
        |r| r.get(0),
    )
    .optional()
    .map_err(Error::from)
}

/// List every `documents.id` owned (via `source_files`) by `source_id` —
/// used by `comemory unindex` to explicitly purge `document_fts` rows
/// before deleting the owning `source_roots` row: the FK cascade removes
/// `source_files`/`documents`/`document_chunks`, but `document_fts` is a
/// virtual table with no FK, so its rows need this id list to clean up
/// independently.
pub fn document_ids_for_source(conn: &Connection, source_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT d.id FROM documents d \
           JOIN source_files sf ON sf.id = d.source_file_id \
          WHERE sf.source_id = ?1",
    )?;
    let ids = stmt
        .query_map(params![source_id], |r| r.get(0))?
        .collect::<std::result::Result<Vec<String>, _>>()?;
    Ok(ids)
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
