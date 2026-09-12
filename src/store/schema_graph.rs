//! Declared schema — the relation graph: the `edges` table (typed, weighted
//! `src → dst` rows with a composite primary key) and the `code_ref` version
//! anchors behind explicit code references. Both took toolu-orm 0.6.0's
//! table-level `#[primary_key(…)]` (#65).

use toolu_orm::core::column::{Integer, Text};
use toolu_orm::table;

/// `edges`: every relation in the graph — memory ↔ memory, memory → file /
/// symbol, and the mined `co_changed` / `imports` / `co_activated` code
/// edges. The `rel` CHECK is the closed list of kinds v13 last extended.
#[table(name = "edges")]
#[primary_key(src_kind, src_id, dst_kind, dst_id, rel)]
#[index("idx_edges_src", src_kind, src_id, rel)]
#[index("idx_edges_dst", dst_kind, dst_id, rel)]
pub struct Edges {
    /// Source node kind (`memory`, `file`, `symbol`, …).
    #[column(not_null)]
    pub src_kind: Text,
    /// Source node id.
    #[column(not_null)]
    pub src_id: Text,
    /// Destination node kind.
    #[column(not_null)]
    pub dst_kind: Text,
    /// Destination node id.
    #[column(not_null)]
    pub dst_id: Text,
    /// Relation kind.
    #[column(
        not_null,
        check = "rel IN ('in_repo','authored_by','tagged','references_file','references_symbol','relates_to','supersedes','conflicts_with','derived_from','co_changed','imports','co_activated','member_of_source','references_document')"
    )]
    pub rel: Text,
    /// Accumulated weight (v6).
    #[column(not_null, default = "1")]
    pub weight: Integer,
    /// RFC3339 time.
    #[column(not_null)]
    pub created_at: Text,
}

/// `code_ref`: the blob OID / commit / branch a memory's explicit code
/// reference was pinned at, so `search --json` can classify it
/// `fresh | stale | ghost | unpinned`.
#[table(name = "code_ref")]
#[primary_key(memory_id, rel, dst_id)]
#[index("idx_code_ref_dst", dst_id, rel)]
pub struct CodeRef {
    /// Owning memory id.
    #[column(not_null)]
    pub memory_id: Text,
    /// `references_file` or `references_symbol`.
    #[column(not_null, check = "rel IN ('references_file','references_symbol')")]
    pub rel: Text,
    /// `<repo>:<path>[:<symbol>]`.
    #[column(not_null)]
    pub dst_id: Text,
    /// HEAD-tree blob OID at save; `NULL` = unpinned.
    pub pinned_blob: Text,
    /// HEAD SHA at save; `NULL` when no repo.
    pub pinned_commit: Text,
    /// Advisory branch name.
    pub branch: Text,
    /// RFC3339 time.
    #[column(not_null)]
    pub created_at: Text,
}
