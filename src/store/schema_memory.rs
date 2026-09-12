//! Declared schema — the memory leg: the `memories` mirror table, its
//! `memory_tags` side table, and the virtual tables `memory_fts` (FTS5,
//! `porter identifier` tokenizer since v4) and `memory_vec` (`vec0`, 1024
//! dims, cosine).

use toolu_orm::core::column::{Integer, Real, Text, Vector};
use toolu_orm::{fts5_table, table, vec0_table};

/// `memories`: frontmatter + body mirror keyed by memory id, plus the
/// access-tracking, SimHash and PageRank columns later migrations appended.
/// The two `WHERE deleted_at IS NULL` indexes are the soft-delete filter
/// every live-memory scan leans on.
#[table(name = "memories")]
#[index("idx_memories_repo", repo, where = "deleted_at IS NULL")]
#[index("idx_memories_kind", kind, where = "deleted_at IS NULL")]
#[index("idx_memories_updated", updated_at)]
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
