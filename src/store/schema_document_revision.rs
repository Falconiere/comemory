//! Declared schema — document replication (#253): the pulled revision cache
//! and the mapping that gives a local document a portable name.
//!
//! Separate from [`super::schema_documents`], whose tables hang off
//! `source_roots` by foreign key — a registered filesystem path a machine that
//! pulled a document does not have. Keeping the two sets apart is what makes
//! "an import never touches a local row" structural rather than remembered.

use toolu_orm::core::column::{Integer, Text};
use toolu_orm::{fts5_table, table};

/// `remote_document`: the revision of one shared document this machine holds.
///
/// Keyed by canonical repo plus `shared_id`, which is the digest of that repo
/// and the document's normalized repository-relative path — neither of them a
/// machine path. Two machines that indexed the same file from different
/// checkouts therefore agree on this key, which is the whole point.
///
/// ONE row per document, replaced wholesale: the last complete accepted
/// revision wins, and the one before it is gone rather than retained. There is
/// deliberately no `state` column, unlike `code_generation`. A code generation
/// is staged locally before upload and so needs a lifecycle; a pulled document
/// has no local staging (`document_share` is the local half) and an incomplete
/// upload never reaches acceptance at all — its parts sit in
/// `replica_staged_part`. What makes a revision appear whole is the acceptance
/// transaction: this row, its chunks, its links and its FTS rows commit
/// together or not at all. A `state` column here could only ever hold one
/// reachable value.
#[table(name = "remote_document")]
#[primary_key(repo, shared_id)]
#[index("idx_remote_document_path", repo, path)]
pub struct RemoteDocument {
    /// Canonical repo — the identity the workspace binding resolved, never a
    /// local alias, so a receiver without the checkout can still match it.
    #[column(not_null)]
    pub repo: Text,
    /// 32 lowercase hex chars over `repo` and `path`.
    #[column(not_null)]
    pub shared_id: Text,
    /// Normalized repository-relative path, forward slashes, never absolute.
    #[column(not_null)]
    pub path: Text,
    /// The document's title as the extractor found it.
    #[column(not_null)]
    pub title: Text,
    /// Which extractor produced the chunks — the four
    /// `documents::document::DocumentFormat` variants.
    #[column(not_null, check = "format IN ('txt', 'markdown', 'html', 'delimited')")]
    pub format: Text,
    /// The sender's `documents.revision_hash` for this revision.
    #[column(not_null)]
    pub revision_hash: Text,
    /// How many chunks the revision has — the count a completeness check
    /// compares the assembled parts against.
    #[column(not_null)]
    pub chunk_count: Integer,
    /// When this machine accepted the revision it now holds. Replaced with the
    /// revision, so it dates THIS text rather than the document's first
    /// appearance — which is what a reader citing a passage needs.
    #[column(not_null)]
    pub accepted_at: Text,
}

/// `remote_document_chunk`: the passages of one pulled revision.
///
/// The same shape `document_chunks` has, minus anything local: there is no
/// `document_id` here because a pulled revision has no local document row to
/// point at.
#[table(name = "remote_document_chunk")]
#[primary_key(repo, shared_id, ordinal)]
pub struct RemoteDocumentChunk {
    /// Canonical repo.
    #[column(not_null)]
    pub repo: Text,
    /// Owning revision.
    #[column(not_null)]
    pub shared_id: Text,
    /// Position within the document, from zero.
    #[column(not_null)]
    pub ordinal: Integer,
    /// Heading breadcrumb, joined with ` > `; empty when the format has none.
    #[column(not_null, default = "''")]
    pub heading_path: Text,
    /// First character offset of the passage.
    #[column(not_null)]
    pub char_start: Integer,
    /// Last character offset of the passage.
    #[column(not_null)]
    pub char_end: Integer,
    /// First line of the passage (1-based).
    #[column(not_null)]
    pub line_start: Integer,
    /// Last line of the passage (1-based, inclusive).
    #[column(not_null)]
    pub line_end: Integer,
    /// 64-bit SimHash of the passage.
    #[column(not_null)]
    pub simhash: Integer,
    /// The passage itself.
    #[column(not_null)]
    pub text: Text,
}

/// `remote_document_link`: the resolvable links one pulled revision carries.
///
/// Local links live as `references_document` rows in `edges`, derived after
/// the index transaction. These are deliberately NOT written there: `edges` is
/// a local table, and an import may not write one. They are read for
/// provenance and resolution instead.
#[table(name = "remote_document_link")]
#[primary_key(repo, shared_id, ordinal, target)]
pub struct RemoteDocumentLink {
    /// Canonical repo.
    #[column(not_null)]
    pub repo: Text,
    /// Owning revision.
    #[column(not_null)]
    pub shared_id: Text,
    /// Which chunk the link was found in.
    #[column(not_null)]
    pub ordinal: Integer,
    /// The link target, as a normalized repository-relative path.
    #[column(not_null)]
    pub target: Text,
}

/// `remote_document_fts`: BM25 index over pulled passages.
///
/// Its own table rather than an `origin` column on `document_fts`: writing
/// pulled text into the index local indexing owns would make an import mutate
/// local state, and would lose which side a hit came from. The document search
/// leg reads both and prefers the local row.
#[fts5_table(name = "remote_document_fts", tokenize = "identifier")]
pub struct RemoteDocumentFts {
    /// Canonical repo, stored but not searchable.
    #[column(unindexed)]
    pub repo: Text,
    /// Owning revision, stored but not searchable.
    #[column(unindexed)]
    pub shared_id: Text,
    /// Chunk ordinal, stored but not searchable.
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

/// `document_share`: what a local document is called when it is shared.
///
/// One row per LOCAL document id, which is never rewritten — this is the
/// explicit mapping that lets a portable name exist alongside the local one
/// rather than replacing it. The unique index over `(repo, shared_id)` is what
/// makes two local documents normalizing onto one shared name a refusal
/// instead of a silent merge of unrelated files.
///
/// `blocked_reason` is set when the secret scan refused the revision: the
/// document stays local and an operator can see why.
#[table(name = "document_share")]
#[primary_key(document_id)]
#[unique_index("uq_document_share_shared", repo, shared_id)]
pub struct DocumentShare {
    /// The local `documents.id`, unchanged. The cascade is the whole
    /// lifecycle: a portable name means nothing without the document it
    /// names, and both local deletion paths (`documents::tombstone` and
    /// `unindex`) drop the parent row, so neither can leave a share behind.
    #[column(not_null, references = "documents(id)", on_delete = "cascade")]
    pub document_id: Text,
    /// Canonical repo this document shares under.
    #[column(not_null)]
    pub repo: Text,
    /// 32 lowercase hex chars over `repo` and `path`.
    #[column(not_null)]
    pub shared_id: Text,
    /// Normalized repository-relative path.
    #[column(not_null)]
    pub path: Text,
    /// Why the secret scan refused to share it; `NULL` when it did not.
    pub blocked_reason: Text,
    /// When this mapping was last written.
    #[column(not_null)]
    pub updated_at: Text,
}
