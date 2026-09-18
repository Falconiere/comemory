//! The `memories` scan behind `GET /api/v1/trash`: every soft-deleted row,
//! newest deletion first. On-disk join and day-countdown math stay in
//! `domains::memories::trash` (spec
//! `docs/toolu/specs/2026-09-07-store-layer-chokepoint-design.md`).

use rusqlite::Connection;

use super::{
    orm,
    schema_memory::{Memories, memories},
};
use crate::prelude::*;
use toolu_orm::core::query_column::CommonOps;

/// The `memories` columns `domains::memories::trash` reads, before the on-disk join.
pub struct DeletedMemoryRow {
    /// 8-hex memory id.
    pub id: String,
    /// Stored body (title is derived from this).
    pub body: String,
    /// Canonical lowercase kind string.
    pub kind: String,
    /// Owning repo, or `None`.
    pub repo: Option<String>,
    /// RFC 3339 timestamp the memory was soft-deleted at.
    pub deleted_at: String,
}

/// Every soft-deleted `memories` row, ordered newest deletion first with the
/// id as a stable tie-breaker (so paging is deterministic when a batch of
/// memories was deleted in the same run).
pub fn deleted_memories(conn: &Connection) -> Result<Vec<DeletedMemoryRow>> {
    let query = Memories::select()
        .columns_typed(&[
            &memories::id,
            &memories::body,
            &memories::kind,
            &memories::repo,
            &memories::deleted_at,
        ])
        .filter(memories::deleted_at.is_not_null())
        .order_by(memories::deleted_at.desc())
        .order_by(memories::id.asc());
    orm::query_all(conn, query.to_sql(), |r| {
        Ok(DeletedMemoryRow {
            id: r.get(0)?,
            body: r.get(1)?,
            kind: r.get(2)?,
            repo: r.get(3)?,
            deleted_at: r.get(4)?,
        })
    })
}

#[cfg(test)]
#[path = "tests/trash_list.rs"]
mod tests;
