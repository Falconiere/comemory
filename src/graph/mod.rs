//! kuzu-backed property graph for the memory layer.
//!
//! [`schema`] holds the DDL applied at open time, [`upsert`] exposes the
//! `MERGE`-based writes used by the save pipeline, and [`query`] hangs
//! read-only traversals off the same [`Graph`] handle.

pub mod cross_link;
pub mod query;
pub mod schema;
pub mod upsert;

pub use upsert::Graph;
