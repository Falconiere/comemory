//! SQLite-backed graph helpers for the memory layer.
//!
//! [`edges`] holds the typed-edge upsert/query API, [`cross_link`]
//! extracts file/symbol references from memory bodies, [`pagerank`]
//! computes deterministic node importance over weighted edges, and
//! [`cochange`] mines git history for files that change together. The
//! previous kuzu-backed `schema`/`upsert`/`query` modules were removed
//! in v0.2.

pub mod cochange;
pub mod cross_link;
pub mod edges;
pub mod pagerank;
