//! The file-level code-connection graph as a value: PageRank-weighted file
//! nodes plus their `imports` / `co_changed` edges, full or windowed.
//!
//! [`query`](super::query) and [`nodes`](super::nodes) build these; the CLI's
//! JSON / DOT / HTML writers in `output::graph` and the `/api/v1/graph`
//! handlers each render the same value, so no transport owns the shape (#170).

use serde::Serialize;

/// One graph node: a single source file, keyed by its canonical graph id
/// (`file:<repo>:<path>`). `rank` is the materialized PageRank projected
/// onto the file; `symbols` counts its top-level (non-chunk) symbols.
#[derive(Serialize, Clone, Debug)]
pub struct Node {
    /// Canonical graph id, `file:<repo>:<path>`.
    pub id: String,
    /// Display label — the repo-relative path.
    pub label: String,
    /// Owning repo label.
    pub repo: String,
    /// Materialized PageRank score (`code_symbols.rank_score`); `0.0` for
    /// files that appear only as a dangling edge endpoint.
    pub rank: f64,
    /// Count of top-level symbols indexed in this file.
    pub symbols: u32,
    /// Distinct live memories referencing this file — by a `references_file`
    /// edge, or by a `references_symbol` edge pointing at one of its
    /// symbols. `0` for a file the code index has never seen.
    pub memories: u64,
    /// Blob OID recorded for this file at index time (`indexed_files`).
    /// `None` for an unindexed edge endpoint, or a file indexed before blob
    /// pinning.
    pub blob: Option<String>,
}

/// One directed graph edge between two file nodes.
#[derive(Serialize, Clone, Debug)]
pub struct Edge {
    /// Source node id (`file:<repo>:<path>`).
    pub src: String,
    /// Destination node id (`file:<repo>:<path>`).
    pub dst: String,
    /// Relation kind: `imports` or `co_changed`.
    pub rel: String,
    /// Edge weight (accumulated co-change count; `1` for imports).
    pub weight: i64,
}

/// The full exportable graph.
#[derive(Serialize, Debug)]
pub struct CodeGraph {
    /// File nodes, sorted by id for deterministic output.
    pub nodes: Vec<Node>,
    /// Directed edges, ordered `weight DESC, rel, src, dst`.
    pub edges: Vec<Edge>,
}

/// A paginated graph: one self-contained subgraph window plus its cursor
/// metadata. Bespoke (not `Page<T>`) because a graph is two coupled
/// collections, not a flat list. The **edge** dimension is paginated; `nodes`
/// is derived to hold exactly the windowed edges' endpoints (no dangling, no
/// extras). JSON:
/// `{ nodes, edges, limit, offset, total, has_more }` (`total` = edge count).
#[derive(Serialize, Debug)]
pub struct GraphPage {
    /// The endpoints of the windowed `edges` (and only those), sorted by id.
    pub nodes: Vec<Node>,
    /// The windowed edges, in `weight DESC, rel, src, dst` order.
    pub edges: Vec<Edge>,
    /// Requested edge-window size. `0` is the sentinel for "all" (no slicing).
    pub limit: usize,
    /// Number of edges skipped before this window started.
    pub offset: usize,
    /// Total number of edges matching the scope filters (pre-window).
    pub total: usize,
    /// Whether edges exist beyond this window (`offset + edges.len() < total`).
    pub has_more: bool,
}

impl GraphPage {
    /// Build a `GraphPage` from a window of edges (already sliced in SQL),
    /// their derived `nodes`, and the cursor metadata. `total` is the count of
    /// all edges matching the scope filters; `has_more` is derived here from
    /// the same window math [`crate::utilities::pagination::Page::from_slice`] uses so
    /// the two envelopes agree (Binding Rule 1).
    pub fn new(graph: CodeGraph, limit: usize, offset: usize, total: usize) -> Self {
        let has_more = offset.saturating_add(graph.edges.len()) < total;
        Self {
            nodes: graph.nodes,
            edges: graph.edges,
            limit,
            offset,
            total,
            has_more,
        }
    }
}

#[cfg(test)]
#[path = "tests/code_graph.rs"]
mod tests;
