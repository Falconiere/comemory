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

/// `code_ref` side table: version-anchor store for explicit code references.
pub mod code_ref;
pub mod code_row;
pub mod connection;
pub mod embed;
pub mod fts;
/// Memory-leg FTS5 ladder (strict → relaxed → subtoken → expanded).
pub mod fts_memory;
pub mod memory_list;
pub mod memory_meta;
pub mod memory_row;
pub mod migrate;
pub mod schema;
pub mod tokenizer;
pub mod vector;

/// `?,?,...,?` — `n` comma-joined SQL placeholders for one `IN (...)` clause.
pub(crate) fn qmarks(n: usize) -> String {
    vec!["?"; n].join(",")
}

/// Inclusive `created_at` window restricting a memory query.
///
/// Bounds are pre-normalized ISO-8601 strings compared through SQLite
/// `datetime()`, so mixed stored precision cannot invert the order the way
/// a lexicographic compare would. A `None` bound leaves that side open, so
/// [`CreatedWindow::default`] reproduces an unfiltered query exactly. The
/// borrow-only pair (rather than two parameters) keeps the query helpers
/// within clippy's argument budget and `store` independent of `retrieval`,
/// whose `scope::TimeScope` owns the equivalent strings.
#[derive(Debug, Clone, Copy, Default)]
pub struct CreatedWindow<'a> {
    /// Inclusive lower bound: keep rows created at or after this instant.
    pub since: Option<&'a str>,
    /// Inclusive upper bound: keep rows created at or before this instant.
    pub cutoff: Option<&'a str>,
}
