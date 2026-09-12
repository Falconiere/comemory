//! Declared schema — the document domain: the `source_roots` /
//! `source_files` registry mirror, the `documents` table, and its
//! `document_fts` chunk index. `document_chunks` stays hand-SQL until
//! toolu-orm can express a composite primary key (#65).

use toolu_orm::core::column::{Integer, Text};
use toolu_orm::{fts5_table, table};

/// `source_roots`: the SQLite mirror of `sources.toml`, one row per
/// registered file or directory root.
#[table(name = "source_roots")]
pub struct SourceRoots {
    /// 32-hex lowercase (128-bit) root id.
    #[column(primary_key, not_null)]
    pub id: Text,
    /// Canonicalized absolute path; unique across the registry.
    #[column(not_null, unique)]
    pub canonical_path: Text,
    /// `file` or `dir`.
    #[column(not_null, check = "kind IN ('file','dir')")]
    pub kind: Text,
    /// Repo label, when the root sits inside one.
    pub repo: Text,
    /// `active` or `unreachable`.
    #[column(
        not_null,
        default = "'active'",
        check = "status IN ('active','unreachable')"
    )]
    pub status: Text,
    /// RFC3339 creation time.
    #[column(not_null)]
    pub created_at: Text,
    /// RFC3339 last update.
    #[column(not_null)]
    pub updated_at: Text,
}

/// `source_files`: every file the discovery walk saw under a root, with
/// its classification, fingerprint and indexing status. The live table's
/// `UNIQUE (source_id, relative_path)` is declared as the equivalent unique
/// index.
#[table(name = "source_files")]
#[index("idx_source_files_status", status)]
#[unique_index("uq_source_files_path", source_id, relative_path)]
pub struct SourceFiles {
    /// 32-hex lowercase (128-bit) file id.
    #[column(primary_key, not_null)]
    pub id: Text,
    /// Owning root.
    #[column(not_null, references = "source_roots(id)", on_delete = "cascade")]
    pub source_id: Text,
    /// Path relative to `source_roots.canonical_path`.
    #[column(not_null)]
    pub relative_path: Text,
    /// `document`, `ignored` or `unsupported`.
    #[column(
        not_null,
        check = "classification IN ('document','ignored','unsupported')"
    )]
    pub classification: Text,
    /// Size in bytes from the discovery walk.
    #[column(not_null)]
    pub size: Integer,
    /// Nanoseconds since the UNIX epoch — the fast fingerprint.
    #[column(not_null)]
    pub mtime: Integer,
    /// Content hash; `NULL` until the fingerprint changed and was confirmed.
    pub sha256: Text,
    /// Indexing status.
    #[column(
        not_null,
        default = "'pending'",
        check = "status IN ('pending','indexed','stale','too_large','error','unsupported','deleted')"
    )]
    pub status: Text,
    /// Typed diagnostic; `NULL` when `status` has none.
    pub error: Text,
    /// RFC3339 creation time.
    #[column(not_null)]
    pub created_at: Text,
    /// RFC3339 last update.
    #[column(not_null)]
    pub updated_at: Text,
}

/// `documents`: one row per extracted document, keyed by a 32-hex id and
/// bound 1:1 to the `source_files` row it came from.
#[table(name = "documents")]
pub struct Documents {
    /// 32-hex lowercase (128-bit) document id.
    #[column(primary_key, not_null)]
    pub id: Text,
    /// The registry file this document was extracted from.
    #[column(
        not_null,
        unique,
        references = "source_files(id)",
        on_delete = "cascade"
    )]
    pub source_file_id: Text,
    /// First heading, else the file stem.
    #[column(not_null)]
    pub title: Text,
    /// Repo label, when the source root sits inside one.
    pub repo: Text,
    /// Content identity for rename / edit detection.
    #[column(not_null)]
    pub revision_hash: Text,
    /// RFC3339 creation time.
    #[column(not_null)]
    pub created_at: Text,
    /// RFC3339 last update.
    #[column(not_null)]
    pub updated_at: Text,
}

/// `document_fts`: BM25 index over document chunks.
#[fts5_table(name = "document_fts", tokenize = "identifier")]
pub struct DocumentFts {
    /// Owning document id, stored but not searchable.
    #[column(unindexed)]
    pub document_id: Text,
    /// Chunk ordinal within the document, stored but not searchable.
    #[column(unindexed)]
    pub ordinal: Text,
    /// Document title.
    pub title: Text,
    /// Heading path of the chunk.
    pub headings: Text,
    /// The chunk text.
    pub passage: Text,
    /// Path split into identifier tokens.
    pub path_tokens: Text,
}
