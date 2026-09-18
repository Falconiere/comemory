//! `code_ref` side table: the version-anchor store for explicit code
//! references.
//!
//! Edges in the `edges` table carry only the graph shape (`references_file` /
//! `references_symbol`); the captured anchor (git blob OID + commit + branch)
//! lives here, keyed by `(memory_id, rel, dst_id)`. Rows are rebuilt from
//! frontmatter on every [`materialize`] call, so `comemory rebuild` restores
//! them for free.

use super::{
    orm,
    schema_graph::{CodeRef, code_ref as c},
    schema_memory::memories as m,
};
use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;

use crate::domains::memories::{Ref, References};
use crate::prelude::*;
use crate::store::edges::{self, EdgeKey, REFERENCES_FILE, REFERENCES_SYMBOL};

/// One materialized code reference with its captured version anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeRefRow {
    /// Owning memory id.
    pub memory_id: String,
    /// Edge kind: `references_file` or `references_symbol`.
    pub rel: String,
    /// Qualified target: `<repo>:<path>[:<symbol>]`.
    pub dst_id: String,
    /// Git blob OID captured at save time (HEAD tree); `None` when unpinned.
    pub pinned_blob: Option<String>,
    /// HEAD commit SHA captured at save time; `None` when unpinned.
    pub pinned_commit: Option<String>,
    /// Branch shorthand captured at save time (advisory).
    pub branch: Option<String>,
}

/// Emit reference edges for `refs` and persist their anchors.
///
/// Reference edges mirror the body-derived ones `memory_row::insert` writes
/// from its [`crate::store::MemoryLinks`] input (`memory → file` /
/// `memory → symbol`); the `edges` table dedups via `INSERT OR IGNORE`, so a
/// ref also mentioned in the body collapses to one edge. The anchors are then
/// written to `code_ref` via [`upsert`].
pub fn materialize(
    conn: &Connection,
    memory_id: &str,
    refs: &References,
    created_at: &str,
) -> Result<()> {
    for r in &refs.files {
        emit_edge(conn, memory_id, "file", REFERENCES_FILE, &r.id)?;
    }
    for r in &refs.symbols {
        emit_edge(conn, memory_id, "symbol", REFERENCES_SYMBOL, &r.id)?;
    }
    upsert(conn, memory_id, refs, created_at)
}

/// Insert one reference edge (`memory → file|symbol`).
fn emit_edge(
    conn: &Connection,
    memory_id: &str,
    dst_kind: &str,
    rel: &str,
    dst_id: &str,
) -> Result<()> {
    edges::insert(
        conn,
        EdgeKey {
            src_kind: "memory",
            src_id: memory_id,
            dst_kind,
            dst_id,
            rel,
        },
    )
}

/// Replace every `code_ref` row for `memory_id` with the current `refs` set.
///
/// Full-replace (DELETE then INSERT) so a reference removed on re-save is
/// actually dropped — unlike the additive `edges` table.
pub fn upsert(
    conn: &Connection,
    memory_id: &str,
    refs: &References,
    created_at: &str,
) -> Result<()> {
    orm::execute(
        conn,
        CodeRef::delete()
            .filter(c::memory_id.eq(memory_id))
            .to_sql(),
    )?;
    for r in &refs.files {
        insert_row(conn, memory_id, REFERENCES_FILE, r, created_at)?;
    }
    for r in &refs.symbols {
        insert_row(conn, memory_id, REFERENCES_SYMBOL, r, created_at)?;
    }
    Ok(())
}

/// Insert a single anchor row.
fn insert_row(
    conn: &Connection,
    memory_id: &str,
    rel: &str,
    r: &Ref,
    created_at: &str,
) -> Result<()> {
    orm::execute(
        conn,
        CodeRef::insert()
            .or_replace()
            .set(&c::memory_id, memory_id)
            .set(&c::rel, rel)
            .set(&c::dst_id, r.id.as_str())
            .set(&c::pinned_blob, r.blob.as_deref())
            .set(&c::pinned_commit, r.commit.as_deref())
            .set(&c::branch, r.branch.as_deref())
            .set(&c::created_at, created_at)
            .to_sql(),
    )?;
    Ok(())
}

/// One anchored `code_ref` row from a live memory, as read by the
/// ghost-reference scan behind
/// `domains::maintenance::retention::stale_code::detect`.
pub struct LiveRefRow {
    /// Owning (live) memory id.
    pub memory_id: String,
    /// Qualified target: `<repo>:<path>[:<symbol>]`.
    pub dst_id: String,
    /// Git blob OID captured at save time; `None` when unpinned.
    pub pinned_blob: Option<String>,
}

/// Every `code_ref` row of relation `rel` attached to a LIVE
/// (`deleted_at IS NULL`) memory, ordered `(memory_id, dst_id)`.
pub fn for_rel_live(conn: &Connection, rel: &str) -> Result<Vec<LiveRefRow>> {
    orm::query_all(
        conn,
        CodeRef::select()
            .column_expr(&c::memory_id.qualified(), "memory_id")
            .column_expr(&c::dst_id.qualified(), "dst_id")
            .column_expr(&c::pinned_blob.qualified(), "pinned_blob")
            .join("memories", m::id.equals(&c::memory_id))
            .filter(m::deleted_at.is_null())
            .filter(c::rel.eq(rel))
            .order_by(c::memory_id.asc())
            .order_by(c::dst_id.asc())
            .to_sql(),
        |row| {
            Ok(LiveRefRow {
                memory_id: row.get(0)?,
                dst_id: row.get(1)?,
                pinned_blob: row.get(2)?,
            })
        },
    )
}

/// Load every code reference attached to `memory_id`, ordered `rel, dst_id`.
pub fn for_memory(conn: &Connection, memory_id: &str) -> Result<Vec<CodeRefRow>> {
    orm::query_all(
        conn,
        CodeRef::select()
            .columns_typed(&[
                &c::memory_id,
                &c::rel,
                &c::dst_id,
                &c::pinned_blob,
                &c::pinned_commit,
                &c::branch,
            ])
            .filter(c::memory_id.eq(memory_id))
            .order_by(c::rel.asc())
            .order_by(c::dst_id.asc())
            .to_sql(),
        |row| {
            Ok(CodeRefRow {
                memory_id: row.get(0)?,
                rel: row.get(1)?,
                dst_id: row.get(2)?,
                pinned_blob: row.get(3)?,
                pinned_commit: row.get(4)?,
                branch: row.get(5)?,
            })
        },
    )
}

#[cfg(test)]
#[path = "tests/code_ref.rs"]
mod tests;
