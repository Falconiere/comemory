//! `document_share` row CRUD — what a local document is called upstream.
//!
//! The local `documents.id` is never rewritten; this table carries the
//! portable name beside it. The unique index over `(repo, shared_id)` is the
//! rule, not a hint: two local documents that normalize onto one shared name
//! are unrelated files, and merging them would lose one of them.

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
/// `(repo, shared_id)`.
///
/// The read below exists only to name the holder in that message. What
/// actually guarantees the rule is `uq_document_share_shared`, so a writer
/// that raced past the read still loses: the constraint violation is mapped to
/// the same refusal rather than escaping as a driver error.
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
    .map_err(|e| unique_violation(e, share))?;
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
        .map_err(|e| unique_violation(e, share))?;
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

/// Turn `uq_document_share_shared` into the same refusal a read would have
/// produced, so a racing writer is answered rather than handed a driver error.
fn unique_violation(error: Error, wanted: &Share) -> Error {
    let is_unique = matches!(
        &error,
        Error::Sqlite(rusqlite::Error::SqliteFailure(inner, _))
            if inner.code == rusqlite::ErrorCode::ConstraintViolation
    );
    if is_unique {
        return Error::Conflict(format!(
            "another document already shares {} as {}",
            wanted.path, wanted.shared_id
        ));
    }
    error
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

/// Forget one document's portable name — its local row is going away.
///
/// # Errors
/// Propagates SQLite failures.
pub fn forget(conn: &Connection, document_id: &str) -> Result<()> {
    orm::execute(
        conn,
        DocumentShare::delete()
            .filter(col::document_id.eq(document_id))
            .to_sql(),
    )?;
    Ok(())
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

#[cfg(test)]
#[path = "tests/document_share.rs"]
mod tests;
