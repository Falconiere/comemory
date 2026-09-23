//! The pulled document cache: the revision of a shared document this machine
//! holds, its passages, its links and its own FTS index.
//!
//! Every write here is wholesale for ONE document. There is no partial update
//! and no retained history: the last accepted revision is the one the tables
//! hold, and `replace_revision` deletes what was there before writing what
//! arrived. The caller owns the transaction, which is what makes a revision
//! appear all at once — a reader between a half-written revision and the rest
//! of it would see passages from two different texts.
//!
//! Nothing in this module touches a local table. That is the property the
//! separate table set exists to make structural rather than remembered.

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;

use super::orm;
use super::schema_document_revision::{
    RemoteDocument, RemoteDocumentChunk, RemoteDocumentFts, RemoteDocumentLink,
    remote_document as col, remote_document_chunk as chunk_col, remote_document_fts as fts_col,
    remote_document_link as link_col,
};
use crate::prelude::*;

/// One pulled revision's header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    /// Canonical repository.
    pub repo: String,
    /// 32 lowercase hex chars over `repo` and `path`.
    pub shared_id: String,
    /// Normalized repository-relative path.
    pub path: String,
    /// Title as the sender's extractor found it.
    pub title: String,
    /// One of `txt`, `markdown`, `html`, `delimited`.
    pub format: String,
    /// The sender's `documents.revision_hash`.
    pub revision_hash: String,
    /// How many chunks this revision has.
    pub chunk_count: i64,
}

/// One pulled passage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Position within the document, from zero.
    pub ordinal: i64,
    /// Heading breadcrumb, joined with ` > `.
    pub heading_path: String,
    /// Character range into the sender's normalized text.
    pub char_range: (i64, i64),
    /// Inclusive 1-based line range.
    pub line_range: (i64, i64),
    /// 64-bit SimHash of the passage.
    pub simhash: i64,
    /// The passage text.
    pub text: String,
}

/// One pulled link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// The chunk the link was found in.
    pub ordinal: i64,
    /// Repository-relative target.
    pub target: String,
}

/// Replace everything this machine holds for one document with `revision`.
///
/// The delete-then-write is the whole contract: a revision with fewer chunks
/// than its predecessor must not leave the extra passages behind, answering
/// search with text the sender no longer has.
///
/// # Errors
/// Propagates SQLite failures.
pub fn replace_revision(
    tx: &Connection,
    revision: &Revision,
    chunks: &[Chunk],
    links: &[Link],
    at: &str,
) -> Result<()> {
    purge(tx, &revision.repo, &revision.shared_id)?;
    orm::execute(
        tx,
        RemoteDocument::insert()
            .set(&col::repo, revision.repo.as_str())
            .set(&col::shared_id, revision.shared_id.as_str())
            .set(&col::path, revision.path.as_str())
            .set(&col::title, revision.title.as_str())
            .set(&col::format, revision.format.as_str())
            .set(&col::revision_hash, revision.revision_hash.as_str())
            .set(&col::chunk_count, revision.chunk_count)
            .set(&col::accepted_at, at)
            .to_sql(),
    )?;
    for chunk in chunks {
        write_passage(tx, revision, chunk)?;
    }
    for link in links {
        orm::execute(
            tx,
            RemoteDocumentLink::insert()
                .set(&link_col::repo, revision.repo.as_str())
                .set(&link_col::shared_id, revision.shared_id.as_str())
                .set(&link_col::ordinal, link.ordinal)
                .set(&link_col::target, link.target.as_str())
                .to_sql(),
        )?;
    }
    Ok(())
}

/// One passage and the row that makes it searchable.
///
/// Written together because they describe the same passage: an FTS row without
/// its chunk matches text nothing can explain, and a chunk without its FTS row
/// is text search cannot reach.
///
/// `path_tokens` carries the raw path — the `identifier` tokenizer splits `/`,
/// `.`, `-` and camelCase itself, so pre-lowercasing would destroy those
/// boundaries, the same reason `document_fts::insert` passes it verbatim.
fn write_passage(tx: &Connection, revision: &Revision, chunk: &Chunk) -> Result<()> {
    orm::execute(
        tx,
        RemoteDocumentChunk::insert()
            .set(&chunk_col::repo, revision.repo.as_str())
            .set(&chunk_col::shared_id, revision.shared_id.as_str())
            .set(&chunk_col::ordinal, chunk.ordinal)
            .set(&chunk_col::heading_path, chunk.heading_path.as_str())
            .set(&chunk_col::char_start, chunk.char_range.0)
            .set(&chunk_col::char_end, chunk.char_range.1)
            .set(&chunk_col::line_start, chunk.line_range.0)
            .set(&chunk_col::line_end, chunk.line_range.1)
            .set(&chunk_col::simhash, chunk.simhash)
            .set(&chunk_col::text, chunk.text.as_str())
            .to_sql(),
    )?;
    orm::execute(
        tx,
        RemoteDocumentFts::insert()
            .set(&fts_col::repo, revision.repo.as_str())
            .set(&fts_col::shared_id, revision.shared_id.as_str())
            .set(&fts_col::ordinal, chunk.ordinal.to_string().as_str())
            .set(&fts_col::title, revision.title.as_str())
            .set(&fts_col::headings, chunk.heading_path.as_str())
            .set(&fts_col::passage, chunk.text.as_str())
            .set(&fts_col::path_tokens, revision.path.as_str())
            .to_sql(),
    )?;
    Ok(())
}

/// Every table one pulled document's rows live in, children before parents.
///
/// `remote_document_fts` is a virtual table with no foreign key, so its rows
/// are deleted explicitly — the same obligation `document_fts` has. The other
/// three are listed rather than cascaded because they all share one key, which
/// is also why one loop can clear them.
const PULLED_TABLES: [&str; 4] = [
    "remote_document_fts",
    "remote_document_chunk",
    "remote_document_link",
    "remote_document",
];

/// Forget everything this machine holds for one document — what a tombstone
/// does, and what [`replace_revision`] does before writing.
///
/// # Errors
/// Propagates SQLite failures.
pub fn purge(tx: &Connection, repo: &str, shared_id: &str) -> Result<()> {
    for table in PULLED_TABLES {
        // The table name comes from the const above, never from a caller.
        tx.execute(
            &format!("DELETE FROM {table} WHERE repo = ?1 AND shared_id = ?2"),
            rusqlite::params![repo, shared_id],
        )?;
    }
    Ok(())
}

/// The revision this machine holds for one document, if any.
///
/// # Errors
/// Propagates SQLite failures.
pub fn revision(conn: &Connection, repo: &str, shared_id: &str) -> Result<Option<Revision>> {
    orm::query_optional(
        conn,
        RemoteDocument::select()
            .columns_typed(&[
                &col::repo,
                &col::shared_id,
                &col::path,
                &col::title,
                &col::format,
                &col::revision_hash,
                &col::chunk_count,
            ])
            .filter(col::repo.eq(repo))
            .filter(col::shared_id.eq(shared_id))
            .to_sql(),
        |r| {
            Ok(Revision {
                repo: r.get(0)?,
                shared_id: r.get(1)?,
                path: r.get(2)?,
                title: r.get(3)?,
                format: r.get(4)?,
                revision_hash: r.get(5)?,
                chunk_count: r.get(6)?,
            })
        },
    )
}

/// A child row of one pulled document, read by [`all`].
///
/// Both queries take `(repo, shared_id)` as their two parameters, because
/// every table here is keyed that way.
pub trait PulledRow: Sized {
    /// The query, taking `?1` repo and `?2` shared_id.
    const SQL: &'static str;

    /// Read one row of [`Self::SQL`]'s column list.
    fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self>;
}

impl PulledRow for Chunk {
    const SQL: &'static str = "SELECT ordinal, heading_path, char_start, char_end, line_start, \
                                      line_end, simhash, text \
                                 FROM remote_document_chunk \
                                WHERE repo = ?1 AND shared_id = ?2 \
                                ORDER BY ordinal";

    fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            ordinal: row.get(0)?,
            heading_path: row.get(1)?,
            char_range: (row.get(2)?, row.get(3)?),
            line_range: (row.get(4)?, row.get(5)?),
            simhash: row.get(6)?,
            text: row.get(7)?,
        })
    }
}

impl PulledRow for Link {
    const SQL: &'static str = "SELECT ordinal, target FROM remote_document_link \
                                WHERE repo = ?1 AND shared_id = ?2 \
                                ORDER BY ordinal, target";

    fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            ordinal: row.get(0)?,
            target: row.get(1)?,
        })
    }
}

/// Every child row of one pulled document, in its own order.
///
/// # Errors
/// Propagates SQLite failures.
pub fn all<T: PulledRow>(conn: &Connection, repo: &str, shared_id: &str) -> Result<Vec<T>> {
    let mut statement = conn.prepare(T::SQL)?;
    let rows = statement
        .query_map([repo, shared_id], T::read)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
#[path = "tests/remote_document.rs"]
mod tests;
