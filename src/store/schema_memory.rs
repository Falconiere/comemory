//! Declared schema — the memory leg: the `memories` mirror table, its
//! `memory_tags` side table, and the virtual tables `memory_fts` (FTS5,
//! `porter identifier` tokenizer since v4), `memory_substring` (trigrams),
//! and `memory_vec` (`vec0`, 1024 dims, cosine).
//!
//! Plus the two write-lifecycle tables added in #251: `memory_write_intent`,
//! the crash-recovery marker for a write in flight, and
//! `memory_needs_embedding`, the backlog of memories whose imported vector was
//! refused. Both belong to the memory leg rather than to
//! [`super::schema_replica`]: neither is part of the replication journal, and
//! both outlive any one replication session.

use toolu_orm::core::column::{Integer, Real, Text, Vector};
use toolu_orm::{fts5_table, table, vec0_table};

#[table(name = "memories")]
#[index("idx_memories_repo", repo, where = "deleted_at IS NULL")]
#[index("idx_memories_kind", kind, where = "deleted_at IS NULL")]
#[index("idx_memories_updated", updated_at)]
#[index(
    "idx_memories_created",
    desc(created_at),
    id,
    where = "deleted_at IS NULL"
)]
#[index(
    "idx_memories_repo_created",
    repo,
    desc(created_at),
    id,
    where = "deleted_at IS NULL"
)]
#[index(
    "idx_memories_kind_created",
    kind,
    desc(created_at),
    id,
    where = "deleted_at IS NULL"
)]
#[index(
    "idx_memories_repo_kind_created",
    repo,
    kind,
    desc(created_at),
    id,
    where = "deleted_at IS NULL"
)]
/// Memory mirror with ranking metadata and indexes for filters and creation order.
pub struct Memories {
    /// 8-hex prefix of `sha256(body)`.
    #[column(primary_key)]
    pub id: Text,
    /// Filename slug.
    #[column(not_null)]
    pub slug: Text,
    /// Memory kind, one of the six frontmatter values.
    #[column(
        not_null,
        check = "kind IN ('decision','bug','convention','discovery','pattern','note')"
    )]
    pub kind: Text,
    /// Repo label, if any.
    pub repo: Text,
    /// Author, if any.
    pub author: Text,
    /// Quality 1..=5, default 3.
    #[column(not_null, default = "3", check = "quality BETWEEN 1 AND 5")]
    pub quality: Integer,
    /// Frontmatter schema version.
    #[column(not_null, default = "1")]
    pub schema: Integer,
    /// `sha256(body.trim_end())`.
    #[column(not_null)]
    pub content_hash: Text,
    /// Markdown body.
    #[column(not_null)]
    pub body: Text,
    /// RFC3339 creation time.
    #[column(not_null)]
    pub created_at: Text,
    /// RFC3339 last update.
    #[column(not_null)]
    pub updated_at: Text,
    /// Soft-delete stamp; `NULL` = live.
    pub deleted_at: Text,
    /// Markdown path relative to the data dir.
    #[column(not_null)]
    pub md_path: Text,
    /// Access tracking (v4).
    #[column(not_null, default = "0")]
    pub access_count: Integer,
    /// Last access, RFC3339 (v4).
    pub last_accessed: Text,
    /// 64-bit SimHash bit pattern of the body (v4).
    #[column(not_null, default = "0")]
    pub simhash: Integer,
    /// Materialized memory-graph PageRank (v11).
    #[column(not_null, default = "0.0")]
    pub rank_score: Real,
}

/// `memory_tags`: one row per (memory, tag).
#[table(name = "memory_tags")]
#[primary_key(memory_id, tag)]
#[index("idx_memory_tags_tag", tag)]
pub struct MemoryTags {
    /// Owning memory; rows go with it.
    #[column(not_null, references = "memories(id)", on_delete = "cascade")]
    pub memory_id: Text,
    /// The tag.
    #[column(not_null)]
    pub tag: Text,
}

/// `memory_fts`: lexical index over memory body + tags. Not contentless —
/// FTS5 stores its own copy so hits render without joining `memories`.
#[fts5_table(name = "memory_fts", tokenize = "porter identifier")]
pub struct MemoryFts {
    /// Owning memory id, stored but not searchable.
    #[column(unindexed)]
    pub memory_id: Text,
    /// Markdown body.
    pub body: Text,
    /// Space-joined tags.
    pub tags: Text,
}

#[fts5_table(
    name = "memory_substring",
    tokenize = "trigram",
    content = "memories",
    content_rowid = "rowid",
    columnsize = 0
)]
/// Trigram postings maintained by triggers; body text is read from `memories.body`.
pub struct MemorySubstring {
    /// External `memories.body` text, indexed without storing a second body copy.
    pub body: Text,
}

/// `memory_vec`: BYO memory embeddings. The `1024` literal is the
/// authoritative dim — `vec0` bakes it into the vtab at migration time and
/// `store::embed::dim_guard` refuses anything else.
#[vec0_table(name = "memory_vec")]
pub struct MemoryVec {
    /// Owning memory id.
    #[column(primary_key)]
    pub memory_id: Text,
    /// Unit-length embedding; `score = 1.0 - distance` recovers cosine.
    #[column(dim = 1024, distance_metric = "cosine")]
    pub embedding: Vector,
}

/// `memory_write_intent`: the one outstanding "a memory write is in flight"
/// row, written before the markdown moves and cleared in the same transaction
/// as the mirror.
///
/// The crash window this closes is the gap between
/// [`crate::domains::memories::MemoryStore::write_atomic`] renaming the file
/// into place and the mirror transaction committing. A process killed there
/// leaves a memory on disk that the database has never seen and that owes no
/// upload; the intent is what lets the next open notice and finish it.
///
/// One row per memory on purpose: a second write to the same id supersedes the
/// first, and a finished write clears it, so the table holds only work in
/// flight rather than a history.
#[table(name = "memory_write_intent")]
pub struct MemoryWriteIntent {
    /// The memory id the write is for.
    #[column(primary_key)]
    pub entity_key: Text,
    /// `write` for a save/update/restore, `delete` for a soft delete.
    #[column(not_null, check = "kind IN ('write', 'delete')")]
    pub kind: Text,
    /// Absolute markdown path the write was placing — the path
    /// `memories::recover` reads directly, not a data-dir-relative one.
    #[column(not_null)]
    pub md_path: Text,
    /// The operation the finished write owes the journal.
    #[column(not_null)]
    pub operation_id: Text,
    /// RFC3339 time the intent was recorded.
    #[column(not_null)]
    pub started_at: Text,
}

/// `memory_needs_embedding`: memories whose text is stored but whose vector is
/// not, with the reason it was refused.
///
/// An imported embedding is only usable when it came from the model this
/// engine queries with, at the dimension its `vec0` table was built for.
/// Refusing the vector must never refuse the memory, so the text lands and the
/// id is recorded here for `doctor` to report and `reembed` to drain.
#[table(name = "memory_needs_embedding")]
#[index("idx_memory_needs_embedding_reason", reason, recorded_at)]
pub struct MemoryNeedsEmbedding {
    /// The memory still owing a vector.
    #[column(primary_key)]
    pub memory_id: Text,
    /// Why: `absent`, `model` or `dims`.
    #[column(not_null, check = "reason IN ('absent', 'model', 'dims')")]
    pub reason: Text,
    /// The model that arrived, when one did.
    pub model: Text,
    /// The dimension that arrived, when one did.
    pub dims: Integer,
    /// RFC3339 time the refusal was recorded.
    #[column(not_null)]
    pub recorded_at: Text,
}
