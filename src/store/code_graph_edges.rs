//! The dynamic, paginated file→file edge query behind `comemory graph` /
//! `GET /api/v1/graph`: `edges` filtered by relation set, weight floor, and
//! an optional repo scope, windowed by `(limit, offset)`, plus the `total`
//! count of edges matching the same filters (pre-window) so the caller can
//! compute an exact `has_more`.
//!
//! Kept out of [`crate::store::edges`] — the CRUD + graph-algorithm home —
//! because the dynamic `WHERE`-builder plus this query's own row mapping
//! would push that file past the 300-line ceiling, the same split already
//! made for [`crate::store::edges_retrieval`]. Node aggregation for the same
//! command lives in the sibling [`crate::store::code_graph_nodes`].

use rusqlite::Connection;

use crate::prelude::*;
use crate::store::edges::{GraphEdgeRow, file_node_prefix};

/// Bound filter + window parameters for [`fetch_page`].
pub struct EdgeQuery<'a> {
    /// `edges.rel` values to include (e.g. `["imports"]` or both kinds).
    pub rels: &'a [&'a str],
    /// Restrict both edge endpoints to this repo's `file:<repo>:` prefix.
    pub repo: Option<&'a str>,
    /// Drop `co_changed` edges below this accumulated weight (`imports`
    /// edges always carry weight 1 and are never dropped by this floor).
    pub min_weight: i64,
    /// Row cap; `0` means "all" (`LIMIT -1`).
    pub limit: usize,
    /// Rows to skip before the window starts.
    pub offset: usize,
}

/// Fetch a `(limit, offset)` window of file→file edges for the selected
/// relations, scoped to one repo's source side and dropping low-weight
/// `co_changed` links, plus the `total` count of edges matching those same
/// scope filters (pre-window). Edges sort by the stable `weight DESC, rel
/// ASC, src_id ASC, dst_id ASC` order, so a bounded export keeps the
/// strongest links deterministically.
pub fn fetch_page(conn: &Connection, q: &EdgeQuery<'_>) -> Result<(Vec<GraphEdgeRow>, usize)> {
    let (where_clause, mut binds) = where_clause_and_binds(q.rels, q.repo, q.min_weight);
    // The COUNT carries only the filter params — never the window.
    let count_sql = format!("SELECT count(*) FROM edges{where_clause}");
    let mut count_stmt = conn.prepare(&count_sql)?;
    let count: i64 = count_stmt.query_row(
        rusqlite::params_from_iter(binds.iter().map(std::convert::AsRef::as_ref)),
        |r| r.get(0),
    )?;
    let total = usize::try_from(count).unwrap_or(0);

    // SQLite forbids a bare `OFFSET`, so `limit == 0` ("all") uses `LIMIT -1`.
    let limit_param: i64 = if q.limit == 0 {
        -1
    } else {
        i64::try_from(q.limit).unwrap_or(i64::MAX)
    };
    binds.push(Box::new(limit_param));
    binds.push(Box::new(i64::try_from(q.offset).unwrap_or(i64::MAX)));
    let sql = format!(
        "SELECT src_id, dst_id, rel, weight FROM edges{where_clause} \
          ORDER BY weight DESC, rel ASC, src_id ASC, dst_id ASC LIMIT ? OFFSET ?"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(
            rusqlite::params_from_iter(binds.iter().map(std::convert::AsRef::as_ref)),
            map_edge,
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok((rows, total))
}

/// Map one `edges` row into a [`GraphEdgeRow`].
fn map_edge(r: &rusqlite::Row<'_>) -> rusqlite::Result<GraphEdgeRow> {
    Ok(GraphEdgeRow {
        src_id: r.get(0)?,
        dst_id: r.get(1)?,
        rel: r.get(2)?,
        weight: r.get(3)?,
    })
}

/// The `WHERE` clause (relation set + weight floor + optional repo scope)
/// and its bound params, `min_weight` as `?1` and `repo` as `?2`.
fn where_clause_and_binds(
    rels: &[&str],
    repo: Option<&str>,
    min_weight: i64,
) -> (String, Vec<Box<dyn rusqlite::ToSql>>) {
    let in_list = rels
        .iter()
        .map(|r| format!("'{r}'"))
        .collect::<Vec<_>>()
        .join(",");
    let mut where_clause = format!(
        " WHERE rel IN ({in_list}) \
            AND (rel <> 'co_changed' OR weight >= ?1)"
    );
    let mut binds: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(min_weight)];
    if let Some(r) = repo {
        // Both endpoints share the `file:<repo>:` prefix gate, so SQLite
        // rejects cross-repo edges directly.
        where_clause.push_str(
            " AND substr(src_id, 1, length(?2)) = ?2 AND substr(dst_id, 1, length(?2)) = ?2",
        );
        binds.push(Box::new(file_node_prefix(r)));
    }
    (where_clause, binds)
}

#[cfg(test)]
#[path = "tests/code_graph_edges.rs"]
mod tests;
