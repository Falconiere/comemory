//! SQLite-backed graph helpers for the memory layer.
//!
//! [`edges`] holds the typed-edge upsert/query API, [`cross_link`]
//! extracts file/symbol references from memory bodies, and [`pagerank`]
//! computes deterministic node importance over weighted edges. The
//! previous kuzu-backed `schema`/`upsert`/`query` modules were removed
//! in v0.2.

pub mod cross_link;
pub mod edges;
pub mod pagerank;
