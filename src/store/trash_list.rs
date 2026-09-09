//! The `memories` scan behind `GET /api/v1/trash`: every soft-deleted row,
//! newest deletion first. On-disk join and day-countdown math stay in
//! `api::trash` (spec
//! `docs/toolu/specs/2026-09-07-store-layer-chokepoint-design.md`).

use rusqlite::Connection;

use crate::prelude::*;

/// The `memories` columns `api::trash` reads, before the on-disk join.
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
    let mut stmt = conn.prepare(
        "SELECT id, body, kind, repo, deleted_at FROM memories \
          WHERE deleted_at IS NOT NULL ORDER BY deleted_at DESC, id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(DeletedMemoryRow {
            id: r.get(0)?,
            body: r.get(1)?,
            kind: r.get(2)?,
            repo: r.get(3)?,
            deleted_at: r.get(4)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

#[cfg(test)]
#[path = "tests/trash_list.rs"]
mod tests;
