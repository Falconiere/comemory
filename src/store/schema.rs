//! Declared table registry for migration generation and schema fidelity tests.
//! Every application table is declared in a `schema_*` sibling; only SQLite's
//! internal bookkeeping is excluded. Runtime apply lives in `store::migrate`.

use toolu_orm::core::schema::SchemaRegistry;
use toolu_orm::core::table::TableSchema;

use super::schema_code::{CodeFts, CodeSymbols, CodeVec, IndexedFiles, RepoMarker};
use super::schema_core::{EdgeFts, SchemaMeta};
use super::schema_documents::{DocumentChunks, DocumentFts, Documents, SourceFiles, SourceRoots};
use super::schema_graph::{CodeRef, Edges};
use super::schema_history::{EvalRuns, GcRuns, IndexFailures, IndexRuns};
use super::schema_learning::{
    BanditArms, CandidateJudgments, CandidateObservations, CandidateQueryObservations,
    CodeFeedback, Feedback, FeedbackEvents, QueryExpansions, RetrievalLog,
};
use super::schema_memory::{Memories, MemoryFts, MemorySubstring, MemoryTags, MemoryVec};
use super::schema_sync::{SyncBinding, SyncLog, SyncState};

/// Every table `registry()` declares, by name, sorted. The fidelity test
/// asserts the registry equals this list, so adding a struct without
/// listing it here (or vice versa) fails loudly.
pub const DECLARED_TABLES: &[&str] = &[
    "bandit_arms",
    "candidate_judgments",
    "candidate_observations",
    "candidate_query_observations",
    "code_feedback",
    "code_fts",
    "code_ref",
    "code_symbols",
    "code_vec",
    "document_chunks",
    "document_fts",
    "documents",
    "edge_fts",
    "edges",
    "eval_runs",
    "feedback",
    "feedback_events",
    "gc_runs",
    "index_failures",
    "index_runs",
    "indexed_files",
    "memories",
    "memory_fts",
    "memory_substring",
    "memory_tags",
    "memory_vec",
    "query_expansions",
    "repo_marker",
    "retrieval_log",
    "schema_meta",
    "source_files",
    "source_roots",
    "sync_binding",
    "sync_log",
    "sync_state",
];

/// The declared schema as a toolu-orm registry — the input to `run_generate`
/// and to the fidelity test. Sorted by table name inside
/// `SchemaRegistry::from_tables`.
#[must_use]
pub fn registry() -> SchemaRegistry {
    SchemaRegistry::from_tables(vec![
        BanditArms::table_def(),
        CandidateJudgments::table_def(),
        CandidateObservations::table_def(),
        CandidateQueryObservations::table_def(),
        CodeFeedback::table_def(),
        CodeFts::table_def(),
        CodeRef::table_def(),
        CodeSymbols::table_def(),
        CodeVec::table_def(),
        DocumentChunks::table_def(),
        DocumentFts::table_def(),
        Documents::table_def(),
        EdgeFts::table_def(),
        Edges::table_def(),
        EvalRuns::table_def(),
        Feedback::table_def(),
        FeedbackEvents::table_def(),
        GcRuns::table_def(),
        IndexFailures::table_def(),
        IndexRuns::table_def(),
        IndexedFiles::table_def(),
        Memories::table_def(),
        MemoryFts::table_def(),
        MemorySubstring::table_def(),
        MemoryTags::table_def(),
        MemoryVec::table_def(),
        QueryExpansions::table_def(),
        RepoMarker::table_def(),
        RetrievalLog::table_def(),
        SchemaMeta::table_def(),
        SourceFiles::table_def(),
        SourceRoots::table_def(),
        SyncBinding::table_def(),
        SyncLog::table_def(),
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
