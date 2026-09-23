//! `document_share` row CRUD — what a local document is called upstream.
//!
//! The local `documents.id` is never rewritten; this table carries the
//! portable name beside it. The unique index over `(repo, shared_id)` is the
//! rule, not a hint: two local documents that normalize onto one shared name
//! are unrelated files, and merging them would lose one of them.
//!
//! There is no deletion function here on purpose. `document_id` is declared
//! `REFERENCES documents(id) ON DELETE CASCADE`, so the row goes when the
//! document does — which is what both local deletion paths already do
//! (`document::writer::tombstone` deletes the `documents` row, `unindex`
//! deletes the `source_roots` row above it). A hand-called purge would be one
//! more thing for a third deletion path to forget.

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;

use super::orm;
use super::schema_document_revision::{DocumentShare, document_share as col};
use crate::prelude::*;

/// One local document's portable name, and why it is not shared if it is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share {
    /// The local `documents.id`.
    pub document_id: String,
    /// Canonical repository.
    pub repo: String,
    /// 32 lowercase hex chars over `repo` and `path`.
    pub shared_id: String,
    /// Normalized repository-relative path.
    pub path: String,
    /// Why the secret scan refused it; `None` when it did not.
    pub blocked_reason: Option<String>,
}

/// Record (or refresh) one document's portable name.
///
/// # Errors
/// [`Error::Conflict`] when another document already claims that
/// `(repo, shared_id)`; [`Error::NotFound`] when `document_id` names no
/// `documents` row.
///
/// The read below exists only to name the holder in that message. What
/// actually guarantees the rule is `uq_document_share_shared`, so a writer
/// that raced past the read still loses: see [`constraint_violation`] for how
/// each constraint becomes an answer rather than a driver error.
pub fn record(conn: &Connection, share: &Share, at: &str) -> Result<()> {
    if let Some(other) = by_shared_id(conn, &share.repo, &share.shared_id)?
        && other.document_id != share.document_id
    {
        return Err(collision(&other, share));
    }
    let updated = orm::execute(
        conn,
        DocumentShare::update()
            .set(&col::repo, share.repo.as_str())
            .set(&col::shared_id, share.shared_id.as_str())
            .set(&col::path, share.path.as_str())
            .set(&col::blocked_reason, share.blocked_reason.as_deref())
            .set(&col::updated_at, at)
            .filter(col::document_id.eq(share.document_id.as_str()))
            .to_sql(),
    )
    .map_err(|e| constraint_violation(e, share))?;
    if updated == 0 {
        orm::execute(
            conn,
            DocumentShare::insert()
                .set(&col::document_id, share.document_id.as_str())
                .set(&col::repo, share.repo.as_str())
                .set(&col::shared_id, share.shared_id.as_str())
                .set(&col::path, share.path.as_str())
                .set(&col::blocked_reason, share.blocked_reason.as_deref())
                .set(&col::updated_at, at)
                .to_sql(),
        )
        .map_err(|e| constraint_violation(e, share))?;
    }
    Ok(())
}

/// The refusal two unrelated files sharing one name earn.
fn collision(holder: &Share, wanted: &Share) -> Error {
    Error::Conflict(format!(
        "document {} already shares {} as {}; {} normalizes onto the same name",
        holder.document_id, holder.path, wanted.shared_id, wanted.path
    ))
}

/// Turn this table's two constraints into answers rather than driver errors.
///
/// The extended code is what distinguishes them, and it has to: mapping every
/// `ConstraintViolation` to the name collision would report "already shares"
/// for a missing parent row, which is a different problem with a different
/// fix. Anything else propagates untouched.
fn constraint_violation(error: Error, wanted: &Share) -> Error {
    let Error::Sqlite(rusqlite::Error::SqliteFailure(inner, _)) = &error else {
        return error;
    };
    match inner.extended_code {
        // `uq_document_share_shared`: the refusal a read would have produced,
        // so a writer that raced past that read still loses.
        rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE => Error::Conflict(format!(
            "another document already shares {} as {}",
            wanted.path, wanted.shared_id
        )),
        // `document_id REFERENCES documents(id)`: a portable name may not
        // outlive, or precede, the document it names.
        rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY => Error::NotFound(format!(
            "no document {} to share as {}",
            wanted.document_id, wanted.shared_id
        )),
        _ => error,
    }
}

/// One local document's portable name, if it has one.
///
/// # Errors
/// Propagates SQLite failures.
pub fn by_document(conn: &Connection, document_id: &str) -> Result<Option<Share>> {
    one(conn, |select| {
        select.filter(col::document_id.eq(document_id))
    })
}

/// Whichever document claims `(repo, shared_id)`, if any.
///
/// # Errors
/// Propagates SQLite failures.
pub fn by_shared_id(conn: &Connection, repo: &str, shared_id: &str) -> Result<Option<Share>> {
    one(conn, |select| {
        select
            .filter(col::repo.eq(repo))
            .filter(col::shared_id.eq(shared_id))
    })
}

/// The columns [`row`] reads, in order.
const COLUMNS: [&dyn toolu_orm::core::query_column::ColumnRef; 5] = [
    &col::document_id,
    &col::repo,
    &col::shared_id,
    &col::path,
    &col::blocked_reason,
];

/// One row narrowed by `filter`.
fn one<F>(conn: &Connection, narrow: F) -> Result<Option<Share>>
where
    F: FnOnce(toolu_orm::query::select::SelectBuilder) -> toolu_orm::query::select::SelectBuilder,
{
    let select = narrow(DocumentShare::select().columns_typed(&COLUMNS));
    orm::query_optional(conn, select.to_sql(), row)
}

/// One `document_share` row of [`COLUMNS`].
fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Share> {
    Ok(Share {
        document_id: r.get(0)?,
        repo: r.get(1)?,
        shared_id: r.get(2)?,
        path: r.get(3)?,
        blocked_reason: r.get(4)?,
    })
}

/// Every blocked document under one source's tree, as `(path, reason)`.
///
/// Joined through `source_files` so a source can report what it is holding
/// back without the documents capability having to know this table's shape.
///
/// # Errors
/// Propagates SQLite failures.
pub fn blocked_for_source(conn: &Connection, source_id: &str) -> Result<Vec<(String, String)>> {
    let mut statement = conn.prepare(
        "SELECT s.path, s.blocked_reason FROM document_share s \
           JOIN documents d ON d.id = s.document_id \
           JOIN source_files f ON f.id = d.source_file_id \
          WHERE f.source_id = ?1 AND s.blocked_reason IS NOT NULL \
          ORDER BY s.path",
    )?;
    let rows = statement
        .query_map([source_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
#[path = "tests/document_share.rs"]
mod tests;
