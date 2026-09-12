//! The declared schema: `registry()` — every table whose live DDL is stated
//! as a toolu-orm `#[table]` / `#[fts5_table]` / `#[vec0_table]` struct in
//! the `schema_*.rs` siblings — and `DECLARED_TABLES`, the same set by name.
//!
//! The registry is what `examples/migrations.rs` diffs against
//! `migrations/<newest>.snapshot.json` to generate the next migration, and
//! what the colocated fidelity test proves identical to the database the
//! frozen `migrations/*.sql` chain builds. Tables toolu-orm 0.5.0 cannot
//! express faithfully (composite primary key / `AUTOINCREMENT` — upstream
//! #65; `DESC` index column — #70, fixed upstream after 0.5.0) are
//! deliberately absent: they are hand-SQL tables, listed in
//! `docs/guides/schema-migrations.md`, and a struct for one of them would
//! fail the fidelity test on exactly the shape toolu-orm cannot render.
//!
//! Runtime apply is unchanged and lives in `store::migrate` — this module
//! never touches a connection.

use toolu_orm::core::schema::SchemaRegistry;
use toolu_orm::core::table::TableSchema;

use super::schema_code::{CodeFts, CodeSymbols, CodeVec, RepoMarker};
use super::schema_core::{EdgeFts, SchemaMeta};
use super::schema_documents::{DocumentFts, Documents, SourceFiles, SourceRoots};
use super::schema_learning::{BanditArms, Feedback, FeedbackEvents, RetrievalLog};
use super::schema_memory::{Memories, MemoryFts, MemoryVec};
use super::schema_sync::{SyncBinding, SyncState};

/// Every table `registry()` declares, by name, sorted. The fidelity test
/// asserts the registry equals this list, so adding a struct without
/// listing it here (or vice versa) fails loudly.
pub const DECLARED_TABLES: &[&str] = &[
    "bandit_arms",
    "code_fts",
    "code_symbols",
    "code_vec",
    "document_fts",
    "documents",
    "edge_fts",
    "feedback",
    "feedback_events",
    "memories",
    "memory_fts",
    "memory_vec",
    "repo_marker",
    "retrieval_log",
    "schema_meta",
    "source_files",
    "source_roots",
    "sync_binding",
    "sync_state",
];

/// The declared schema as a toolu-orm registry — the input to `run_generate`
/// and to the fidelity test. Sorted by table name inside
/// `SchemaRegistry::from_tables`.
#[must_use]
pub fn registry() -> SchemaRegistry {
    SchemaRegistry::from_tables(vec![
        BanditArms::table_def(),
        CodeFts::table_def(),
        CodeSymbols::table_def(),
        CodeVec::table_def(),
        DocumentFts::table_def(),
        Documents::table_def(),
        EdgeFts::table_def(),
        Feedback::table_def(),
        FeedbackEvents::table_def(),
        Memories::table_def(),
        MemoryFts::table_def(),
        MemoryVec::table_def(),
        RepoMarker::table_def(),
        RetrievalLog::table_def(),
        SchemaMeta::table_def(),
        SourceFiles::table_def(),
        SourceRoots::table_def(),
        SyncBinding::table_def(),
        SyncState::table_def(),
    ])
}

#[cfg(test)]
#[path = "tests/schema.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/schema_fidelity.rs"]
mod tests_fidelity;

#[cfg(test)]
#[path = "tests/schema_drift.rs"]
mod tests_drift;
