//! Single-file SQLite-backed storage for comemory v0.2.
//!
//! Replaces the v0.1 fan-out across kuzu (graph), lancedb (vectors),
//! and a manual FTS layer with one `comemory.db` SQLite file. The
//! `sqlite-vec` extension is loaded on every connection for ANN
//! queries; FTS5 is bundled into rusqlite.
//!
//! Module layout matches Task 2.3 of the v0.2 plan:
//! `connection` (open + PRAGMAs + sqlite-vec auto-extension),
//! `embed` (f32 ↔ vec0 BLOB encoding + dim guards),
//! `fts` (FTS5 insert/search helpers for memory/code),
//! `migrate` (versioned schema migrations + `schema_meta`),
//! `schema` (DDL strings for tables/vec0/fts5 vtabs),
//! `vector` (insert/select against `memory_vec` and `code_vec`).
//!
//! Tasks 3–6 of the v0.2 plan flesh out the bodies; Task 2 publishes
//! the skeleton so downstream tasks have stable import paths.

pub mod connection;
pub mod embed;
pub mod fts;
pub mod migrate;
pub mod schema;
pub mod vector;
