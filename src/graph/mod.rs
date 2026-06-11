//! SQLite-backed graph helpers for the memory layer.
//!
//! [`edges`] holds the typed-edge upsert/query API, [`cross_link`]
//! extracts file/symbol references from memory bodies, [`pagerank`]
//! computes deterministic node importance over weighted edges,
//! [`cochange`] mines git history for files that change together,
//! [`imports`] extracts per-language import statements and resolves them
//! to indexed file paths, and [`materialize`] is the `index-code`
//! post-pass that persists mined pairs + resolved imports as edges and
//! projects PageRank onto `code_symbols.rank_score`. The previous
//! kuzu-backed `schema`/`upsert`/`query` modules were removed in v0.2.

pub mod cochange;
pub mod cross_link;
pub mod edges;
pub mod imports;
pub mod materialize;
pub mod pagerank;
