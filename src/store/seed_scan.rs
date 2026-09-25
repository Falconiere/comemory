//! The ordered `memories` scan journal seeding walks — live ids above a
//! resume point.
//!
//! Ordered by `id` rather than by time: seeding must resume exactly where it
//! stopped, and two memories can share a `created_at` while no two share an
//! id.

use rusqlite::Connection;
use toolu_orm::core::expr::Scalar;
use toolu_orm::core::query_column::CommonOps;

use super::orm;
use super::schema_memory::{Memories, memories as col};
use crate::prelude::*;

/// Live memory ids greater than `after`, ascending, capped at `limit`.
///
/// Pass an empty `after` to start from the beginning.
///
/// # Errors
/// Propagates SQLite failures.
pub fn live_memories_after(conn: &Connection, after: &str, limit: usize) -> Result<Vec<String>> {
    let limit = seed_limit(limit)?;
    orm::query_all(
        conn,
        Memories::select()
            .columns_typed(&[&col::id])
            .filter(Scalar::col(&col::id).gt(Scalar::bind(after)))
            .filter(col::deleted_at.is_null())
            .order_by(col::id.asc())
            .limit(limit)
            .to_sql(),
        |r| r.get(0),
    )
}

/// A document not yet claimed by [`crate::store::document_share`] — a
/// candidate for [`crate::domains::sync::replica::seed_documents`].
pub struct UnsharedDocument {
    /// `documents.id`.
    pub document_id: String,
    /// `documents.source_file_id`.
    pub source_file_id: String,
    /// `documents.revision_hash` at the time it was indexed.
    pub revision_hash: String,
}

/// `document_id`s with no `document_share` row, greater than `after`,
/// ascending, capped at `limit`. Hand SQL: the anti-join the walk needs is
/// not one the declared builders express (`docs/guides/runtime-orm.md`).
///
/// # Errors
/// Propagates SQLite failures.
pub fn unshared_documents_after(
    conn: &Connection,
    after: &str,
    limit: usize,
) -> Result<Vec<UnsharedDocument>> {
    let limit = seed_limit(limit)?;
    let mut statement = conn.prepare(
        "SELECT id, source_file_id, revision_hash FROM documents \
          WHERE id > ?1 \
            AND NOT EXISTS (SELECT 1 FROM document_share WHERE document_share.document_id = documents.id) \
          ORDER BY id ASC LIMIT ?2",
    )?;
    let rows = statement
        .query_map((after, limit), |r| {
            Ok(UnsharedDocument {
                document_id: r.get(0)?,
                source_file_id: r.get(1)?,
                revision_hash: r.get(2)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// `limit` as SQLite's `i64`, or a descriptive error for a batch size that
/// cannot be represented.
fn seed_limit(limit: usize) -> Result<i64> {
    i64::try_from(limit).map_err(|_| Error::Other(format!("seed batch not representable: {limit}")))
}

#[cfg(test)]
#[path = "tests/seed_scan.rs"]
mod tests;
