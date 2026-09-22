//! Declared schema — code-index replication (#252): the per-repo generation
//! record and the three tables a pulled projection lands in.
//!
//! Separate from [`super::schema_code`] on purpose — those tables are what a
//! local `index-code` owns, these are what a peer sent. Source search cannot
//! accidentally read a remote row because it is not in the table it reads,
//! rather than because it remembered a filter.
//!
//! A generation is the unit of consistency: activation flips one row and
//! replaces one projection, so a reader never sees half of two heads.

use toolu_orm::core::column::{Integer, Text};
use toolu_orm::table;

/// `code_generation`: one row per generation of one repo's index, local or
/// pulled.
///
/// `origin` is what stops a replication loop: only `local` generations are
/// ever offered upstream, so a projection this machine downloaded can never be
/// re-uploaded as if it were its own work.
///
/// `parent_id` is the generation this one was planned against. The server's
/// accepted order decides the winner, and a generation whose parent is no
/// longer active must be replanned rather than merged — which is what keeps
/// two machines from producing a union of unrelated heads.
#[table(name = "code_generation")]
#[primary_key(repo, generation_id)]
#[index("idx_code_generation_state", repo, state)]
#[index("idx_code_generation_origin", origin, state)]
pub struct CodeGeneration {
    /// Canonical repo label — the one the workspace binding resolves, never a
    /// local alias, so a receiver without the checkout can still match it.
    #[column(not_null)]
    pub repo: Text,
    /// 32 lowercase hex chars, the digest of the canonical payload.
    #[column(not_null)]
    pub generation_id: Text,
    /// The generation this one was planned against; `NULL` for a repo's first.
    pub parent_id: Text,
    /// HEAD commit the index was built at.
    #[column(not_null)]
    pub head: Text,
    /// `repo_marker.last_mined_commit` at build time, when the repo has one.
    pub mined_commit: Text,
    /// `local` for this machine's own index, `sync` for a pulled projection.
    #[column(not_null, check = "origin IN ('local', 'sync')")]
    pub origin: Text,
    /// `staged` until it is complete, `active` once it is, `superseded` after
    /// a later generation replaced it.
    #[column(
        not_null,
        check = "state IN ('staged', 'active', 'superseded')",
        default = "'staged'"
    )]
    pub state: Text,
    /// How many files the manifest names — the count a completeness check
    /// compares the assembled parts against.
    #[column(not_null)]
    pub file_count: Integer,
    /// 64-hex SHA-256 over the canonical manifest, symbols and edges.
    #[column(not_null)]
    pub manifest_digest: Text,
    /// RFC3339 time the generation was recorded.
    #[column(not_null)]
    pub created_at: Text,
    /// RFC3339 time it became `active`; `NULL` while staged.
    pub activated_at: Text,
}

/// `remote_code_file`: one row per file in a pulled generation's manifest.
///
/// The `blob_oid` is the digest the sender indexed at. It is never compared
/// against a local checkout's blob — a pulled generation describes the
/// sender's tree, not this machine's.
#[table(name = "remote_code_file")]
#[primary_key(repo, generation_id, path)]
pub struct RemoteCodeFile {
    /// Canonical repo label.
    #[column(not_null)]
    pub repo: Text,
    /// The generation this row belongs to.
    #[column(not_null)]
    pub generation_id: Text,
    /// Path relative to the repo root.
    #[column(not_null)]
    pub path: Text,
    /// Git blob OID the sender indexed the file at.
    #[column(not_null)]
    pub blob_oid: Text,
}

/// `remote_code_symbol`: the snippet-free symbols of a pulled generation.
///
/// No `snippet` and no `simhash` columns exist here, which is how the
/// no-source-replication rule is enforced structurally rather than by a
/// writer remembering to blank them.
#[table(name = "remote_code_symbol")]
#[primary_key(repo, generation_id, path, symbol, line_start)]
#[index("idx_remote_code_symbol_file", repo, generation_id, path)]
pub struct RemoteCodeSymbol {
    /// Canonical repo label.
    #[column(not_null)]
    pub repo: Text,
    /// The generation this row belongs to.
    #[column(not_null)]
    pub generation_id: Text,
    /// Path relative to the repo root.
    #[column(not_null)]
    pub path: Text,
    /// Qualified symbol name.
    #[column(not_null)]
    pub symbol: Text,
    /// `function` / `struct` / … as the sender's extractor reported it.
    #[column(not_null)]
    pub kind: Text,
    /// `rust` / `typescript` / ….
    #[column(not_null)]
    pub lang: Text,
    /// First line (1-based).
    #[column(not_null)]
    pub line_start: Integer,
    /// Last line (1-based, inclusive).
    #[column(not_null)]
    pub line_end: Integer,
}

/// `remote_code_edge`: a pulled generation's graph projection — resolved
/// `imports` edges and mined `co_changed` pairs, each with the revision it was
/// derived at.
///
/// Weights are REPLACED with the generation, never accumulated across
/// generations. That is what makes a replay idempotent: re-applying the same
/// generation writes the same rows rather than doubling every weight.
#[table(name = "remote_code_edge")]
#[primary_key(repo, generation_id, rel, src_path, dst_path)]
#[index("idx_remote_code_edge_src", repo, generation_id, src_path)]
pub struct RemoteCodeEdge {
    /// Canonical repo label.
    #[column(not_null)]
    pub repo: Text,
    /// The generation this row belongs to.
    #[column(not_null)]
    pub generation_id: Text,
    /// `imports` or `co_changed`.
    #[column(not_null, check = "rel IN ('imports', 'co_changed')")]
    pub rel: Text,
    /// Source file path.
    #[column(not_null)]
    pub src_path: Text,
    /// Target file path.
    #[column(not_null)]
    pub dst_path: Text,
    /// Co-change count, or `1` for a resolved import.
    #[column(not_null, default = "1")]
    pub weight: Integer,
    /// The revision the edge was derived at — the source file's blob OID for
    /// an import, the mined commit for a co-change pair.
    pub anchor: Text,
}
