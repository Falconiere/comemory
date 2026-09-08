//! The SQL behind `api::prune`'s scan and apply phases — everything
//! [`crate::store::prune_signals`] does not already own: the orphan-edge
//! count, the stale-code-file scan, one memory's display fields, and the
//! apply-time cleanup deletes. Report shaping, the soft-delete call, and the
//! transaction itself stay in `api::prune` (spec
//! `docs/toolu/specs/2026-09-07-store-layer-chokepoint-design.md`).
//!
//! **Every `NOT EXISTS` subquery below is correlated on purpose** — each one
//! joins the outer row's `(repo, path)` (or `src_id`/`dst_id`) back into the
//! subquery. An uncorrelated rewrite would match every row the moment ANY
//! `indexed_files` (or `code_symbols`) row exists anywhere, silently turning
//! a per-file/per-edge check into an all-or-nothing one.

use rusqlite::Connection;

use crate::prelude::*;

/// One `memories` row read for [`api::prune`]'s display list: body (for the
/// title), creation stamp, access count, and last-accessed stamp.
pub struct PruneMemoryRow {
    /// The memory's stored body.
    pub body: String,
    /// RFC 3339 `created_at`.
    pub created_at: String,
    /// `memories.access_count`, as stored.
    pub access_count: i64,
    /// `memories.last_accessed`, or `None` if never accessed.
    pub last_accessed: Option<String>,
}

/// Count of `edges` rows sourced at a memory that no longer exists (or was
/// soft-deleted) — the "orphan edges" figure in the scan report.
pub fn count_orphan_memory_edges(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT count(*) FROM edges e \
          WHERE e.src_kind = 'memory' \
            AND NOT EXISTS(SELECT 1 FROM memories m \
                             WHERE m.id = e.src_id AND m.deleted_at IS NULL)",
        [],
        |r| r.get(0),
    )
    .map_err(Error::from)
}

/// `<repo>:<path>` for every distinct `code_symbols` file with no matching
/// `indexed_files` cursor, ordered by repo then path.
///
/// A row that fails to decode is dropped rather than failing the whole scan
/// (`filter_map(Result::ok)`) — preserved verbatim from `api::prune::scan`,
/// a deliberate blanket swallow (spec Non-Goal 3), not introduced by this
/// move.
pub fn stale_code_files(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT repo, path FROM code_symbols \
          WHERE NOT EXISTS(SELECT 1 FROM indexed_files i \
                             WHERE i.repo = code_symbols.repo \
                               AND i.path = code_symbols.path) \
          ORDER BY repo, path",
    )?;
    let rows: Vec<String> = stmt
        .query_map([], |r| {
            let repo: String = r.get(0)?;
            let path: String = r.get(1)?;
            Ok(format!("{repo}:{path}"))
        })?
        .filter_map(std::result::Result::ok)
        .collect();
    Ok(rows)
}

/// The display fields for one memory, by id.
pub fn memory_for_prune(conn: &Connection, id: &str) -> Result<PruneMemoryRow> {
    conn.query_row(
        "SELECT body, created_at, access_count, last_accessed FROM memories WHERE id = ?1",
        [id],
        |r| {
            Ok(PruneMemoryRow {
                body: r.get(0)?,
                created_at: r.get(1)?,
                access_count: r.get(2)?,
                last_accessed: r.get(3)?,
            })
        },
    )
    .map_err(Error::from)
}

/// Delete every `edges` row sourced at a memory that no longer exists (or
/// was soft-deleted) — the write-side twin of
/// [`count_orphan_memory_edges`]'s scan.
pub fn delete_orphan_memory_edges(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM edges WHERE src_kind = 'memory' \
           AND NOT EXISTS(SELECT 1 FROM memories m \
                            WHERE m.id = src_id AND m.deleted_at IS NULL)",
        [],
    )?;
    Ok(())
}

/// Delete the `code_vec` / `code_fts` / `code_symbols` rows for files no
/// longer in `indexed_files` — the write-side twin of [`stale_code_files`].
/// The two virtual tables don't participate in the FK cascade, so their
/// rows are dropped first by the about-to-be-removed `code_symbols.id`.
pub fn purge_stale_code_rows(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM code_vec WHERE symbol_id IN ( \
             SELECT id FROM code_symbols \
              WHERE NOT EXISTS(SELECT 1 FROM indexed_files i \
                                 WHERE i.repo = code_symbols.repo \
                                   AND i.path = code_symbols.path))",
        [],
    )?;
    conn.execute(
        "DELETE FROM code_fts WHERE symbol_id IN ( \
             SELECT id FROM code_symbols \
              WHERE NOT EXISTS(SELECT 1 FROM indexed_files i \
                                 WHERE i.repo = code_symbols.repo \
                                   AND i.path = code_symbols.path))",
        [],
    )?;
    conn.execute(
        "DELETE FROM code_symbols \
          WHERE NOT EXISTS(SELECT 1 FROM indexed_files i \
                             WHERE i.repo = code_symbols.repo \
                               AND i.path = code_symbols.path)",
        [],
    )?;
    Ok(())
}

/// Drop edges that dangle once a file's `code_symbols` rows are purged:
/// `references_symbol` / `references_file` (bare qualified dst) and
/// `co_activated` (the `file:`-prefixed node id from `store::edges`). The
/// read path tolerates a dangling dst, but the count grows every prune
/// cycle without this.
pub fn drop_dangling_edges(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM edges \
          WHERE rel = 'references_symbol' \
            AND NOT EXISTS( \
                SELECT 1 FROM code_symbols cs \
                 WHERE edges.dst_id = cs.repo || ':' || cs.path || ':' || cs.symbol \
            )",
        [],
    )?;
    conn.execute(
        "DELETE FROM edges \
          WHERE rel = 'references_file' \
            AND NOT EXISTS( \
                SELECT 1 FROM code_symbols cs \
                 WHERE edges.dst_id = cs.repo || ':' || cs.path \
            )",
        [],
    )?;
    conn.execute(
        "DELETE FROM edges \
          WHERE rel = 'co_activated' \
            AND NOT EXISTS( \
                SELECT 1 FROM code_symbols cs \
                 WHERE edges.dst_id = 'file:' || cs.repo || ':' || cs.path \
            )",
        [],
    )?;
    Ok(())
}

/// Drop `code_ref` rows left behind once their backing `references_file` /
/// `references_symbol` edge is gone. `code_ref` and `edges` share the same
/// `(memory_id, rel, dst_id)` key for these two relations, so a `code_ref`
/// with no surviving edge is an orphan — exactly the rows
/// [`drop_dangling_edges`] just purged (dangling dst) or that a deleted
/// memory's edge sweep removed.
pub fn drop_orphan_code_refs(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM code_ref \
          WHERE NOT EXISTS( \
              SELECT 1 FROM edges e \
               WHERE e.rel = code_ref.rel \
                 AND e.src_id = code_ref.memory_id \
                 AND e.dst_id = code_ref.dst_id \
          )",
        [],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/prune_apply.rs"]
mod tests;
