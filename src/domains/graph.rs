//! `domains::graph` — deriving, mining, ranking and querying the `edges`
//! relation graph.
//!
//! [`code_graph`][code_graph] is the model both transports serialize.
//! [`cross_link`][cross_link], [`doc_link`][doc_link], [`cochange`][cochange],
//! [`imports`][imports] and [`coactivate`][coactivate] derive and mine edges;
//! [`pagerank`][pagerank], [`materialize`][materialize] and
//! [`memory_rank`][memory_rank] score them; [`derived`][derived] is the one
//! best-effort refresh every write seam calls. [`query`][query] and
//! [`nodes`][nodes] assemble the file-level graph. [`edges`][edges] is the
//! command core both delivery adapters call; [`view`][view] backs `GET /graph`
//! alone (the CLI calls [`query`][query] directly — see its module doc), and
//! [`graph_nodes`][graph_nodes] / [`graph_recompute`][graph_recompute] are
//! console-only, and `search_edit` is `pub(crate)` — reachable from inside the
//! crate only, so it carries no intra-doc link here. Every SQL string stays in
//! [`crate::store::edges`] and its `code_graph_*` / `edge_fts` siblings.
//!
//! A capability's `//!` doc merges with the `///` on its `pub mod` line in
//! `src/domains.rs`, so bare intra-doc links here would resolve in the
//! `domains` scope. Every target below is therefore spelled in full.
//!
//! [coactivate]: crate::domains::graph::coactivate
//! [cochange]: crate::domains::graph::cochange
//! [code_graph]: crate::domains::graph::code_graph
//! [cross_link]: crate::domains::graph::cross_link
//! [derived]: crate::domains::graph::derived
//! [doc_link]: crate::domains::graph::doc_link
//! [edges]: crate::domains::graph::edges
//! [graph_nodes]: crate::domains::graph::graph_nodes
//! [graph_recompute]: crate::domains::graph::graph_recompute
//! [imports]: crate::domains::graph::imports
//! [materialize]: crate::domains::graph::materialize
//! [memory_rank]: crate::domains::graph::memory_rank
//! [nodes]: crate::domains::graph::nodes
//! [pagerank]: crate::domains::graph::pagerank
//! [query]: crate::domains::graph::query
//! [view]: crate::domains::graph::view

/// Commit co-activation reward over a memory's referenced files.
pub mod coactivate;
/// Git co-change mining: files that change together in bounded history.
pub mod cochange;
/// `Node`, `Edge`, `CodeGraph` and `GraphPage` — the exported graph model.
pub mod code_graph;
/// Extract `<repo>:<path>[:<symbol>]` references from a memory body.
pub mod cross_link;
/// Unified best-effort refresh of every derived artifact.
pub mod derived;
/// Deterministic `member_of_source` / `references_document` link deriver.
pub mod doc_link;
/// `comemory edges` / `GET /edges`: lexical search over the relation graph.
pub mod edges;
/// The owned value `comemory edges` produces.
pub mod edges_result;
/// `GET /graph/nodes*`, `GET /graph/snapshot`: listing, detail, neighbors.
pub mod graph_nodes;
/// `POST /graph/recompute`: PageRank re-projection job.
pub mod graph_recompute;
/// Per-language import extraction and module-to-path resolution.
pub mod imports;
/// `index-code` post-pass: mined pairs + imports → edges + projected PageRank.
pub mod materialize;
/// PageRank over the derived memory graph → `memories.rank_score`.
pub mod memory_rank;
/// One-hop undirected file neighborhood over `imports`/`co_changed`.
pub mod neighbors;
/// Node assembly: the windowed-edge endpoint dedup and `build_graph`.
pub mod nodes;
/// Deterministic PageRank over a weighted directed graph.
pub mod pagerank;
/// Relation vocabulary and the edge-window query behind the graph builders.
pub mod query;
/// Search→edit lookback feeding `auto_search_edit` provenance.
pub(crate) mod search_edit;
/// `comemory graph` / `GET /graph`: the full graph or one windowed page.
pub mod view;
