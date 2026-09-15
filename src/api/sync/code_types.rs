//! Wire types for the code-index sync surface — `GET /sync/code/manifest`
//! and `POST /sync/code/import` (code-graph sync design, 2026-09-15).
//!
//! Everything here is snippet-free by construction: a file carries its
//! symbols' names, kinds, languages and line ranges, and the paths it
//! imports — never `code_symbols.snippet`, which is source text. The unit of
//! sync is the file, and `blob_oid` is its digest on both sides.

use serde::{Deserialize, Serialize};

/// One indexed file's identity: its repo-relative path and the git blob OID
/// `index-code` extracted it at. The diff key the client compares against
/// its own `indexed_files` rows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CodeFileRef {
    /// Path relative to the repo root.
    pub path: String,
    /// Git blob OID at index time.
    pub blob_oid: String,
}

/// `GET /sync/code/manifest?repo=` payload — what the workspace holds for
/// one repo label. An unknown label answers an empty file list and no head.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeManifestResponse {
    /// The repo label asked about.
    pub repo: String,
    /// `repo_marker.last_head` — the HEAD the last accepted push was at.
    pub head: Option<String>,
    /// `repo_marker.last_mined_commit` — the co-change set's cursor.
    pub mined_commit: Option<String>,
    /// Every file the workspace holds for `repo`, ascending by path.
    pub files: Vec<CodeFileRef>,
}

/// One top-level symbol of a file, snippet-free.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CodeSymbolWire {
    /// Qualified symbol name.
    pub symbol: String,
    /// `function` / `struct` / … as the extractor reports it.
    pub kind: String,
    /// `rust` / `typescript` / ….
    pub lang: String,
    /// First line (1-based).
    pub line_start: i64,
    /// Last line (1-based, inclusive).
    pub line_end: i64,
}

/// One file of an import batch: its identity, its top-level symbols, and
/// the repo-relative paths its `imports` edges point at.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CodeFileWire {
    /// Path relative to the repo root.
    pub path: String,
    /// Git blob OID at index time.
    pub blob_oid: String,
    /// The file's top-level symbols (chunk children are not carried).
    #[serde(default)]
    pub symbols: Vec<CodeSymbolWire>,
    /// Repo-relative paths this file imports, resolved on the pushing side.
    #[serde(default)]
    pub imports: Vec<String>,
}

/// One mined co-change pair — an undirected edge stored once.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CoChangeWire {
    /// One endpoint, repo-relative.
    pub from: String,
    /// The other endpoint, repo-relative.
    pub to: String,
    /// Number of commits the pair changed together in.
    pub weight: i64,
}

/// `POST /sync/code/import` body — one batch of a repo's projection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeImportRequest {
    /// Repo label every row lands under.
    pub repo: String,
    /// The pushing side's `repo_marker.last_head`; stamped after the batch
    /// applies, so `/repos` can report it.
    #[serde(default)]
    pub head: Option<String>,
    /// The pushing side's co-change mining cursor. Required when `cochange`
    /// is present; stored as the repo's `last_mined_commit`.
    #[serde(default)]
    pub mined_commit: Option<String>,
    /// Files to create or replace (≤ 500 per batch).
    #[serde(default)]
    pub files: Vec<CodeFileWire>,
    /// Paths the workspace holds and the pushing side no longer has.
    #[serde(default)]
    pub removed: Vec<String>,
    /// When present, the repo's whole `co_changed` set — replaced wholesale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cochange: Option<Vec<CoChangeWire>>,
}

/// One entry the import refused, and why. A batch with any rejection
/// writes nothing — see `code_import::run`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CodeImportRejection {
    /// The offending path (a repo-level problem uses `""`).
    pub path: String,
    /// Machine-readable reason (`invalid_path`, `code_quota`, …) followed by
    /// a short detail.
    pub reason: String,
}

/// `POST /sync/code/import` payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeImportResponse {
    /// Files written or confirmed unchanged.
    pub applied: usize,
    /// Paths removed.
    pub removed: usize,
    /// What was refused. Non-empty means the batch was not applied.
    pub rejected: Vec<CodeImportRejection>,
    /// `repo_marker.last_head` after the batch.
    pub head: Option<String>,
}
