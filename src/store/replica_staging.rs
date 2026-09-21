//! `replica_staged_part` row CRUD — the parts of a revision too large for one
//! envelope, and their assembly.
//!
//! Staged parts are ordinary rows in a table nothing else reads: `changes` and
//! `manifest` are built from the feed and the revisions, so an upload that
//! never activated has published nothing.

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;

use super::orm;
use super::schema_replica::{ReplicaStagedPart, replica_staged_part as col};
use crate::prelude::*;

/// What a staging id currently holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagingState {
    /// Parts received so far.
    pub received: i64,
    /// Parts the upload declared.
    pub declared: i64,
}

impl StagingState {
    /// Whether every declared part has arrived.
    #[must_use]
    pub fn complete(&self) -> bool {
        self.declared > 0 && self.received == self.declared
    }
}

/// Store one part, replacing a re-sent part with the same index.
///
/// A re-sent part is a retry, not a second part: the index is the identity.
///
/// # Errors
/// Propagates SQLite failures.
pub fn put_part(
    conn: &Connection,
    staging_id: &str,
    part_index: i64,
    part_count: i64,
    bytes: &str,
    at: &str,
) -> Result<()> {
    let updated = orm::execute(
        conn,
        ReplicaStagedPart::update()
            .set(&col::part_count, part_count)
            .set(&col::bytes, bytes)
            .set(&col::created_at, at)
            .filter(col::staging_id.eq(staging_id))
            .filter(col::part_index.eq(part_index))
            .to_sql(),
    )?;
    if updated > 0 {
        return Ok(());
    }
    orm::execute(
        conn,
        ReplicaStagedPart::insert()
            .set(&col::staging_id, staging_id)
            .set(&col::part_index, part_index)
            .set(&col::part_count, part_count)
            .set(&col::bytes, bytes)
            .set(&col::created_at, at)
            .to_sql(),
    )?;
    Ok(())
}

/// How many parts have arrived for `staging_id`, and how many were declared.
///
/// # Errors
/// Propagates SQLite failures.
pub fn state(conn: &Connection, staging_id: &str) -> Result<StagingState> {
    let row: Option<(i64, Option<i64>)> = orm::query_optional(
        conn,
        ReplicaStagedPart::select()
            .column_expr("COUNT(*)", "received")
            .column_expr("MAX(part_count)", "declared")
            .filter(col::staging_id.eq(staging_id))
            .to_sql(),
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let (received, declared) = row.unwrap_or((0, None));
    Ok(StagingState {
        received,
        declared: declared.unwrap_or(0),
    })
}

/// Concatenate every stored part in index order.
///
/// Returns `None` when a declared part is still missing — the caller must
/// refuse activation rather than publish a truncated revision.
///
/// Completeness is judged from the SAME read that produces the bytes: asking
/// first and reading after would let a part that landed in between decide the
/// two questions differently.
///
/// # Errors
/// Propagates SQLite failures.
pub fn assemble(conn: &Connection, staging_id: &str) -> Result<Option<String>> {
    let parts: Vec<(String, i64)> = orm::query_all(
        conn,
        ReplicaStagedPart::select()
            .columns_typed(&[&col::bytes, &col::part_count])
            .filter(col::staging_id.eq(staging_id))
            .order_by(col::part_index.asc())
            .to_sql(),
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let declared = parts.iter().map(|(_, count)| *count).max().unwrap_or(0);
    let received = i64::try_from(parts.len()).unwrap_or(i64::MAX);
    if declared < 1 || received != declared {
        return Ok(None);
    }
    Ok(Some(
        parts
            .into_iter()
            .map(|(bytes, _)| bytes)
            .collect::<String>(),
    ))
}

/// Drop every part of one upload — after activation, or when it is abandoned.
///
/// # Errors
/// Propagates SQLite failures.
pub fn discard(conn: &Connection, staging_id: &str) -> Result<usize> {
    orm::execute(
        conn,
        ReplicaStagedPart::delete()
            .filter(col::staging_id.eq(staging_id))
            .to_sql(),
    )
}

#[cfg(test)]
#[path = "tests/replica_staging.rs"]
mod tests;
