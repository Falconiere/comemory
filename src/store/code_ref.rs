//! `code_ref` side table: the version-anchor store for explicit code
//! references.
//!
//! Edges in the `edges` table carry only the graph shape (`references_file` /
//! `references_symbol`); the captured anchor (git blob OID + commit + branch)
//! lives here, keyed by `(memory_id, rel, dst_id)`. Rows are rebuilt from
//! frontmatter on every [`materialize`] call, so `comemory rebuild` restores
//! them for free.

use rusqlite::Connection;

use crate::graph::edges::{self, EdgeKey, REFERENCES_FILE, REFERENCES_SYMBOL};
use crate::memory::{Ref, References};
use crate::prelude::*;

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
/// Reference edges mirror [`crate::graph::cross_link::extract_and_emit`]
/// (`memory → file` / `memory → symbol`); the `edges` table dedups via
/// `INSERT OR IGNORE`, so a ref also mentioned in the body collapses to one
/// edge. The anchors are then written to `code_ref` via [`upsert`].
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
    conn.execute("DELETE FROM code_ref WHERE memory_id = ?1", [memory_id])?;
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
    conn.execute(
        "INSERT OR REPLACE INTO code_ref \
         (memory_id, rel, dst_id, pinned_blob, pinned_commit, branch, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![memory_id, rel, r.id, r.blob, r.commit, r.branch, created_at],
    )?;
    Ok(())
}

/// Load every code reference attached to `memory_id`, ordered `rel, dst_id`.
pub fn for_memory(conn: &Connection, memory_id: &str) -> Result<Vec<CodeRefRow>> {
    let mut stmt = conn.prepare(
        "SELECT memory_id, rel, dst_id, pinned_blob, pinned_commit, branch \
         FROM code_ref WHERE memory_id = ?1 ORDER BY rel, dst_id",
    )?;
    let rows = stmt.query_map([memory_id], |row| {
        Ok(CodeRefRow {
            memory_id: row.get(0)?,
            rel: row.get(1)?,
            dst_id: row.get(2)?,
            pinned_blob: row.get(3)?,
            pinned_commit: row.get(4)?,
            branch: row.get(5)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}
