//! Stats subsystem: SQLite-backed retrieval log, per-memory feedback counters,
//! and per-repo indexing markers.

pub mod code_feedback;
pub mod feedback;
pub mod sqlite;

pub use sqlite::StatsDb;
