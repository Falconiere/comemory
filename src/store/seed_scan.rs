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
    let limit = i64::try_from(limit)
        .map_err(|_| Error::Other(format!("seed batch not representable: {limit}")))?;
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

#[cfg(test)]
#[path = "tests/seed_scan.rs"]
mod tests;
