//! Node assembly for the file-level code graph: the pure `(repo, path)`
//! dedup over a windowed edge set, and the [`build_graph`] pass that joins
//! node rows to their edges. The SQL and its row mapping —
//! [`NodeRow`](crate::store::code_graph_nodes::NodeRow),
//! `fetch_nodes`/`fetch_node`, and the `cites_file_predicate` fragment
//! `api::graph_nodes` also reuses — moved to
//! [`crate::store::code_graph_nodes`] (spec
//! `docs/toolu/specs/2026-09-07-store-layer-chokepoint-design.md`).
//!
//! Split out of `cli::graph` when the node query grew the `memories` and
//! `blob` columns the console's selected-node panel needs — the donor file
//! was at the 300-line ceiling. `cli::graph` keeps the CLI surface, the edge
//! fetch, and the two `build_*` entry points; everything about turning a
//! `(repo, path)` pair into a [`Node`] lives here.

use std::collections::{BTreeMap, BTreeSet};

use crate::cli::graph::parse_id;
use crate::output::graph::{CodeGraph, Edge, Node};
use crate::prelude::*;
use crate::store::code_graph_nodes::NodeRow;
use crate::store::edges::file_node_id;
use crate::store::{self, Connection};

/// Fetch one [`NodeRow`] per distinct endpoint file referenced by `edges`, so
/// a paged subgraph carries exactly the nodes its windowed edges touch (and
/// no others). Endpoints whose ids don't parse, or that have no
/// `code_symbols` rows (stale edges), simply produce no row here —
/// [`build_graph`] then materializes them as zero-rank nodes so the edge is
/// never orphaned. Deduping into a `BTreeSet` keeps the node list
/// deterministic; the batched fetch itself lives in
/// [`store::code_graph_nodes::fetch_nodes_for_pairs`].
pub fn fetch_nodes_for_edges(conn: &Connection, edges: &[Edge]) -> Result<Vec<NodeRow>> {
    let pairs: BTreeSet<(String, String)> = edges
        .iter()
        .flat_map(|e| [e.src.as_str(), e.dst.as_str()])
        .filter_map(|id| parse_id(id).map(|(r, p)| (r.to_string(), p.to_string())))
        .collect();
    let pairs: Vec<(String, String)> = pairs.into_iter().collect();
    store::code_graph_nodes::fetch_nodes_for_pairs(conn, &pairs)
}

/// Assemble the [`CodeGraph`] from node rows and edges. Edge endpoints that
/// have no `code_symbols` row (e.g. a stale co-change link to a deleted file)
/// are still materialized as zero-rank nodes so the edge is not orphaned —
/// those carry no blob and no memory count, which is the honest answer for a
/// file the index has never seen.
pub fn build_graph(node_rows: Vec<NodeRow>, edges: Vec<Edge>) -> CodeGraph {
    let mut nodes: BTreeMap<String, Node> = BTreeMap::new();
    for row in node_rows {
        let id = file_node_id(&row.repo, &row.path);
        nodes.insert(
            id.clone(),
            Node {
                id,
                label: row.path,
                repo: row.repo,
                rank: row.rank,
                symbols: row.symbols,
                memories: row.memories,
                blob: row.blob,
            },
        );
    }
    for e in &edges {
        for id in [&e.src, &e.dst] {
            if nodes.contains_key(id) {
                continue;
            }
            if let Some((repo, path)) = parse_id(id) {
                nodes.insert(
                    id.clone(),
                    Node {
                        id: id.clone(),
                        label: path.to_string(),
                        repo: repo.to_string(),
                        rank: 0.0,
                        symbols: 0,
                        memories: 0,
                        blob: None,
                    },
                );
            }
        }
    }
    CodeGraph {
        nodes: nodes.into_values().collect(),
        edges,
    }
}
