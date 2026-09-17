//! The relation vocabulary, and the edge-window query behind the two graph
//! builders. Three public items: `Rel` is the vocabulary, `build_code_graph`
//! and `build_graph_page` are the builders.
//!
//! [`Rel`] is the single relation vocabulary — `comemory graph --rel`,
//! `GET /api/v1/graph`'s `rel` and `GET /api/v1/graph/snapshot`'s
//! `edge_kinds` all resolve through its `ValueEnum` string table, so no second
//! match arm exists in delivery (Binding Rule 1). [`build_code_graph`] returns
//! the whole graph and [`build_graph_page`] one `(limit, offset)` window; both
//! are `pub(crate)` — in-crate callers only, never a library promise — and
//! both transports share them, so they cannot drift. The SQL lives in
//! [`crate::store::code_graph_edges::fetch_page`], where `limit == 0` is the
//! "no window" sentinel it renders as SQLite's `LIMIT -1`.

use clap::ValueEnum;

use crate::domains::graph::code_graph::{CodeGraph, Edge, GraphPage};
use crate::domains::graph::nodes::{build_graph, fetch_nodes_for_edges};
use crate::prelude::*;
use crate::store::Connection;
use crate::store::code_graph_edges::{self, EdgeQuery};
use crate::store::code_graph_nodes::fetch_nodes;

/// Which edge relations to include.
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum Rel {
    /// Both `imports` and `co_changed`.
    All,
    /// Static import edges only.
    Imports,
    /// Git co-change edges only.
    CoChanged,
}

/// Build the file-level [`CodeGraph`] for the selected repo / relations /
/// min-weight, returning the **full** graph (no edge window). Used by the
/// `comemory serve` graph handler's backward-compatible "no params" path.
pub(crate) fn build_code_graph(
    conn: &Connection,
    repo: Option<&str>,
    rel: Rel,
    min_weight: i64,
) -> Result<CodeGraph> {
    let (edges, _total) = fetch_edges(conn, repo, rels_of(rel), min_weight, 0, 0)?;
    let node_rows = fetch_nodes(conn, repo)?;
    Ok(build_graph(node_rows, edges))
}

/// Build the paginated [`GraphPage`] for the selected scope, windowing the
/// edges by `(limit, offset)` and deriving the page's nodes from only those
/// edges' endpoints. Shared by `cli::graph`'s CLI surface and the `comemory serve`
/// graph handler's paginated path so the two cannot drift.
pub(crate) fn build_graph_page(
    conn: &Connection,
    repo: Option<&str>,
    rel: Rel,
    min_weight: i64,
    limit: usize,
    offset: usize,
) -> Result<GraphPage> {
    let (edges, total) = fetch_edges(conn, repo, rels_of(rel), min_weight, limit, offset)?;
    let node_rows = fetch_nodes_for_edges(conn, &edges)?;
    let graph = build_graph(node_rows, edges);
    Ok(GraphPage::new(graph, limit, offset, total))
}

/// The `edges.rel` values selected by a [`Rel`] choice.
fn rels_of(rel: Rel) -> &'static [&'static str] {
    match rel {
        Rel::All => &["co_changed", "imports"],
        Rel::Imports => &["imports"],
        Rel::CoChanged => &["co_changed"],
    }
}

/// Fetch a `(limit, offset)` window of file→file edges for the selected
/// relations, scoped to one repo's source side and dropping low-weight
/// `co_changed` links, plus the `total` count of edges matching those same
/// scope filters (pre-window) so the caller can compute an exact `has_more`.
/// The SQL and its row mapping live in [`code_graph_edges::fetch_page`]; this
/// just shapes the result into the domain's [`Edge`] type.
fn fetch_edges(
    conn: &Connection,
    repo: Option<&str>,
    rels: &[&str],
    min_weight: i64,
    limit: usize,
    offset: usize,
) -> Result<(Vec<Edge>, usize)> {
    let (rows, total) = code_graph_edges::fetch_page(
        conn,
        &EdgeQuery {
            rels,
            repo,
            min_weight,
            limit,
            offset,
        },
    )?;
    let edges = rows
        .into_iter()
        .map(|r| Edge {
            src: r.src_id,
            dst: r.dst_id,
            rel: r.rel,
            weight: r.weight,
        })
        .collect();
    Ok((edges, total))
}

#[cfg(test)]
#[path = "tests/query.rs"]
mod tests;
