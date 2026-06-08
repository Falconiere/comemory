//! Single-file SQLite-backed storage for comemory v0.2.
//!
//! Replaces the v0.1 fan-out across kuzu (graph), lancedb (vectors),
//! and a manual FTS layer with one `comemory.db` SQLite file. The
//! `sqlite-vec` extension is loaded on every connection for ANN
//! queries; FTS5 is bundled into rusqlite.

pub mod connection;
