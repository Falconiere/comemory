//! Stats subsystem: SQLite-backed retrieval log, per-memory feedback counters,
//! and per-repo indexing markers. Also the single home of the
//! `retrieval_log.source` / `feedback_events.target_kind` vocabularies, so
//! writers and readers cannot drift on the literal strings.

pub mod code_feedback;
pub mod feedback;
pub mod sqlite;

pub use sqlite::StatsDb;

/// The `retrieval_log.source` vocabulary: which command originated a
/// logged query. Writers pass these consts; readers (`eval::golden`,
/// `eval::mine`) bind them as SQL parameters instead of inlining the
/// literals.
pub(crate) mod source {
    /// `comemory search` (memory search).
    pub(crate) const SEARCH: &str = "search";
    /// `comemory context` (context bundles).
    pub(crate) const CONTEXT: &str = "context";
    /// `comemory search-code` (code search). Excluded from reformulation
    /// mining and golden-set harvesting — these rows can only earn
    /// code-target feedback.
    pub(crate) const SEARCH_CODE: &str = "search-code";
}

/// The `feedback_events.target_kind` vocabulary: what kind of id the
/// (memory-era-named) `memory_id` column carries for one verdict row.
pub(crate) mod target {
    /// Memory id (8-hex).
    pub(crate) const MEMORY: &str = "memory";
    /// Text-encoded `code_symbols` rowid (see [`crate::stats::code_feedback`]).
    pub(crate) const CODE: &str = "code";
}
