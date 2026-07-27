//! SQLite-backed graph helpers for the memory layer.
//!
//! [`edges`] holds the typed-edge upsert/query API, [`cross_link`]
//! extracts file/symbol references from memory bodies, [`pagerank`]
//! computes deterministic node importance over weighted edges,
//! [`cochange`] mines git history for files that change together,
//! [`imports`] extracts per-language import statements and resolves them
//! to indexed file paths, [`coactivate`] applies the commit co-activation
//! reward (commits touching a memory's referenced files reinforce it; a
//! recent search/context hit upgrades the Beta mint to `auto_search_edit`),
//! [`materialize`] is the `index-code` post-pass that persists mined pairs
//! and resolved imports as edges, then projects PageRank onto
//! `code_symbols.rank_score`, and [`memory_rank`] does the same over the
//! derived memory graph, scoring `memories.rank_score`. The previous
//! kuzu-backed `schema`/`upsert`/`query` modules were removed in v0.2.

pub mod coactivate;
pub mod cochange;
pub mod cross_link;
pub mod edges;
pub mod imports;
pub mod materialize;
/// PageRank over the derived memory graph → `memories.rank_score`.
pub mod memory_rank;
pub mod pagerank;
pub(crate) mod search_edit;
