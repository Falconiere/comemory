//! SQLite-backed edge store. Replaces the v0.1 kuzu writer.

use rusqlite::{params, Connection};

use crate::prelude::*;

/// Addressing tuple for a single directed edge.
///
/// Node identifiers follow the v0.2 convention documented in
/// `src/store/sql/0002_v2_tables.sql`:
/// `memory:<id>`, `file:<repo>:<path>`, `symbol:<symbol_id>`,
/// `repo:<repo>`, `author:<name>`, `tag:<name>`.
#[derive(Clone, Copy)]
pub struct EdgeKey<'a> {
    /// Source node kind (e.g. `"memory"`, `"file"`).
    pub src_kind: &'a str,
    /// Source node identifier.
    pub src_id: &'a str,
    /// Destination node kind.
    pub dst_kind: &'a str,
    /// Destination node identifier.
    pub dst_id: &'a str,
    /// Relation label; must match the `edges.rel` CHECK constraint.
    pub rel: &'a str,
}

/// Insert (or no-op if already present) one edge stamped with the current
/// UTC time.
pub fn insert(conn: &Connection, e: EdgeKey<'_>) -> Result<()> {
    insert_at(conn, e, None)
}

/// Insert (or no-op if already present) one edge, stamped with the given
/// `created_at` string when `Some`, or the current UTC time when `None`.
///
/// The explicit-timestamp form exists for `store::memory_row`, which wipes
/// and re-emits a memory's outgoing edges on every re-save: relation edges
/// (`supersedes` / …) must keep their original `created_at` across the
/// wipe, because `prune::low_value::superseded_rule` compares the target's
/// `last_accessed` against the edge timestamp — a refreshed stamp would
/// re-arm the rule on every re-save of the superseder.
pub fn insert_at(conn: &Connection, e: EdgeKey<'_>, created_at: Option<&str>) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at) \
         VALUES(?1,?2,?3,?4,?5, COALESCE(?6, strftime('%Y-%m-%dT%H:%M:%fZ','now')))",
        params![e.src_kind, e.src_id, e.dst_kind, e.dst_id, e.rel, created_at],
    )?;
    Ok(())
}

/// Outgoing neighbors of `(src_kind, src_id)` following `rel`.
pub fn outgoing(
    conn: &Connection,
    src_kind: &str,
    src_id: &str,
    rel: &str,
) -> Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT dst_kind, dst_id FROM edges \
          WHERE src_kind = ?1 AND src_id = ?2 AND rel = ?3",
    )?;
    let rows = stmt
        .query_map(params![src_kind, src_id, rel], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Transitive `supersedes` chain starting at `start` memory id,
/// depth-bounded by `max_depth`.
///
/// Uses `UNION` (not `UNION ALL`) in the recursive CTE so SQLite deduplicates
/// `(id, depth)` tuples on the fly and terminates immediately when a cycle
/// would re-visit an already-seen node. This prevents exponential blowup on
/// graphs with back-edges (e.g. a→b, b→a).
pub fn supersedes_chain(conn: &Connection, start: &str, max_depth: u32) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE walk(id, depth) AS (
             SELECT ?1, 0
             UNION
             SELECT e.dst_id, w.depth + 1
               FROM edges e
               JOIN walk w ON e.src_id = w.id
              WHERE e.src_kind = 'memory' AND e.dst_kind = 'memory'
                AND e.rel = 'supersedes'
                AND w.depth < ?2
         )
         SELECT id FROM walk WHERE depth > 0 ORDER BY depth",
    )?;
    let rows = stmt
        .query_map(params![start, max_depth as i64], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Delete every edge originating at `(kind, id)` (source side only). Used
/// by the re-insert cleanup in `store::memory_row`: a re-save or rebuild of
/// a memory must refresh the edges it *emits* without destroying incoming
/// edges such as another memory's `supersedes` pointing at it — rebuild
/// replays memories newest-first, so the superseder's edge is already in
/// place when the superseded memory is inserted.
pub fn delete_outgoing(conn: &Connection, kind: &str, id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM edges WHERE src_kind = ?1 AND src_id = ?2",
        params![kind, id],
    )?;
    Ok(())
}

/// Delete every edge touching `(kind, id)`, either side. Used by
/// soft-delete.
pub fn delete_touching(conn: &Connection, kind: &str, id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM edges WHERE (src_kind = ?1 AND src_id = ?2) OR (dst_kind = ?1 AND dst_id = ?2)",
        params![kind, id],
    )?;
    Ok(())
}
