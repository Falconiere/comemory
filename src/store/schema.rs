//! Declared table registry for migration generation and schema fidelity tests.
//! Every application table is declared in a `schema_*` sibling; only SQLite's
//! internal bookkeeping is excluded. Runtime apply lives in `store::migrate`.

use toolu_orm::core::schema::SchemaRegistry;
use toolu_orm::core::table::TableSchema;

use super::schema_code::{CodeFts, CodeSymbols, CodeVec, IndexedFiles, RepoMarker};
use super::schema_code_generation::{
    CodeGeneration, RemoteCodeEdge, RemoteCodeFile, RemoteCodeSymbol,
};
use super::schema_core::{EdgeFts, SchemaMeta};
use super::schema_document_revision::{
    DocumentShare, RemoteDocument, RemoteDocumentChunk, RemoteDocumentFts, RemoteDocumentLink,
};
use super::schema_documents::{DocumentChunks, DocumentFts, Documents, SourceFiles, SourceRoots};
use super::schema_graph::{CodeRef, Edges};
use super::schema_history::{ActivityLog, EvalRuns, GcRuns, IndexFailures, IndexRuns};
use super::schema_learning::{
    BanditArms, CandidateJudgments, CandidateObservations, CandidateQueryObservations,
    CodeFeedback, Feedback, FeedbackEvents, QueryExpansions, RetrievalLog,
};
use super::schema_memory::{
    Memories, MemoryFts, MemoryNeedsEmbedding, MemorySubstring, MemoryTags, MemoryVec,
    MemoryWriteIntent,
};
use super::schema_replica::{
    ReplicaCursor, ReplicaFeed, ReplicaOperation, ReplicaPayload, ReplicaReceipt, ReplicaRevision,
    ReplicaStagedPart, ReplicaStream,
};
use super::schema_sync::{RepositoryApproval, SyncBinding, SyncLog, SyncState};

/// Every table `registry()` declares, by name, sorted. The fidelity test
/// asserts the registry equals this list, so adding a struct without
/// listing it here (or vice versa) fails loudly.
pub const DECLARED_TABLES: &[&str] = &[
    "activity_log",
    "bandit_arms",
    "candidate_judgments",
    "candidate_observations",
    "candidate_query_observations",
    "code_feedback",
    "code_fts",
    "code_generation",
    "code_ref",
    "code_symbols",
    "code_vec",
    "document_chunks",
    "document_fts",
    "document_share",
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
    "memory_needs_embedding",
    "memory_substring",
    "memory_tags",
    "memory_vec",
    "memory_write_intent",
    "query_expansions",
    "remote_code_edge",
    "remote_code_file",
    "remote_code_symbol",
    "remote_document",
    "remote_document_chunk",
    "remote_document_fts",
    "remote_document_link",
    "replica_cursor",
    "replica_feed",
    "replica_operation",
    "replica_payload",
    "replica_receipt",
    "replica_revision",
    "replica_staged_part",
    "replica_stream",
    "repo_marker",
    "repository_approval",
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
        ActivityLog::table_def(),
        BanditArms::table_def(),
        CandidateJudgments::table_def(),
        CandidateObservations::table_def(),
        CandidateQueryObservations::table_def(),
        CodeFeedback::table_def(),
        CodeGeneration::table_def(),
        CodeFts::table_def(),
        CodeRef::table_def(),
        CodeSymbols::table_def(),
        CodeVec::table_def(),
        DocumentChunks::table_def(),
        DocumentFts::table_def(),
        DocumentShare::table_def(),
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
        MemoryNeedsEmbedding::table_def(),
        MemorySubstring::table_def(),
        MemoryTags::table_def(),
        MemoryVec::table_def(),
        MemoryWriteIntent::table_def(),
        QueryExpansions::table_def(),
        RemoteCodeEdge::table_def(),
        RemoteCodeFile::table_def(),
        RemoteCodeSymbol::table_def(),
        RemoteDocument::table_def(),
        RemoteDocumentChunk::table_def(),
        RemoteDocumentFts::table_def(),
        RemoteDocumentLink::table_def(),
        ReplicaCursor::table_def(),
        ReplicaFeed::table_def(),
        ReplicaOperation::table_def(),
        ReplicaPayload::table_def(),
        ReplicaReceipt::table_def(),
        ReplicaRevision::table_def(),
        ReplicaStagedPart::table_def(),
        ReplicaStream::table_def(),
        RepoMarker::table_def(),
        RepositoryApproval::table_def(),
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
