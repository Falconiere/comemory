//! SQLite-backed graph helpers for the memory layer.
//!
//! [`edges`] holds the typed-edge upsert/query API, [`cross_link`]
//! extracts file/symbol references from memory bodies, [`pagerank`]
//! computes deterministic node importance over weighted edges,
//! [`cochange`] mines git history for files that change together,
//! [`imports`] extracts per-language import statements and resolves them
//! to indexed file paths, [`coactivate`] applies the commit co-activation
//! reward (commits touching a memory's referenced files reinforce it), and
//! [`materialize`] is the `index-code` post-pass that persists mined pairs +
//! resolved imports as edges, projects PageRank onto
//! `code_symbols.rank_score`, and runs the co-activation reward. The
//! previous kuzu-backed `schema`/`upsert`/`query` modules were removed in
//! v0.2.

pub mod coactivate;
pub mod cochange;
pub mod cross_link;
pub mod edges;
pub mod imports;
pub mod materialize;
pub mod pagerank;
