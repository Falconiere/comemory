//! Stats subsystem: SQLite-backed retrieval log, per-memory feedback counters,
//! and per-repo indexing markers.
//!
//! The `retrieval_log.source` / `feedback_events.target_kind` /
//! `feedback_events.provenance` vocabularies these tables persist are shared
//! contracts, so they live in `crate::utilities::telemetry` (#166); this
//! module is the staged shell that
//! [#173](https://github.com/Falconiere/comemory/issues/173) moves under
//! `domains::learning`.

pub mod code_feedback;
pub mod feedback;
pub mod sqlite;

pub use sqlite::StatsDb;
