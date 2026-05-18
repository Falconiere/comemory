//! Stats subsystem: SQLite-backed retrieval log, per-memory feedback counters,
//! and per-repo indexing markers.

pub mod feedback;
pub mod sqlite;

pub use feedback::Feedback;
pub use sqlite::StatsDb;
