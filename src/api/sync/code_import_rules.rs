//! Validation for `POST /sync/code/import` — every rule that can be judged
//! without touching the store, so a batch is refused whole before a single
//! row is written.

use std::collections::BTreeSet;

use crate::api::sync::code_types::{
    CoChangeWire, CodeFileWire, CodeImportRejection, CodeImportRequest,
};

/// Files per batch, mirroring the 500-entry cap on memory import.
pub const MAX_FILES: usize = 500;

/// `code_symbols` rows one repo may hold on a workspace — refused with
/// `code_quota` rather than silently truncated.
pub const SYMBOL_QUOTA: usize = 200_000;

/// Why `repo` cannot name a workspace repo, or `None` when it can. Shared
/// by the import and the manifest so the two routes agree on what a label
/// is: non-empty, trimmed, and free of `:` — a file node id is
/// `file:<repo>:<path>` and the first `:` after the kind ends the repo, so
/// a label carrying one would alias another repo's node space (`a:b` + `c`
/// reads as `a` + `b:c`). Local `index-code` never minted such a label;
/// the wire must not either.
pub(crate) fn repo_label_error(repo: &str) -> Option<&'static str> {
    if repo.trim().is_empty() || repo != repo.trim() {
        return Some("invalid_repo: label must be non-empty and trimmed");
    }
    if repo.contains(':') {
        return Some("invalid_repo: label must not contain `:`");
    }
    None
}

/// A repo-relative path a workspace will store: non-empty, relative, and
/// free of empty, `.` and `..` segments — so no node id can name a file
/// outside the tree, and `foo/./bar` cannot alias `foo/bar` past the
/// duplicate-path check.
fn path_ok(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\0')
        && !path
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
}

/// A git object id as libgit2 prints it: 40 lowercase hex chars for SHA-1,
/// 64 for a SHA-256 repository. Anything else cannot be a blob the manifest
/// diff can compare, and would let a caller pin a file to a digest no
/// `index-code` run will ever reproduce.
fn blob_oid_ok(oid: &str) -> bool {
    matches!(oid.len(), 40 | 64) && oid.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn reject(path: &str, reason: impl Into<String>) -> CodeImportRejection {
    CodeImportRejection {
        path: path.to_owned(),
        reason: reason.into(),
    }
}

/// Every rejection the batch earns on its own shape; empty means it may be
/// applied. `existing_symbols` is the repo's `code_symbols` count on the
/// files this batch does NOT replace, so the quota judges the post-batch
/// total.
pub fn validate(req: &CodeImportRequest, existing_symbols: usize) -> Vec<CodeImportRejection> {
    let mut out = Vec::new();
    if let Some(reason) = repo_label_error(&req.repo) {
        out.push(reject("", reason));
    }
    if req.files.len() > MAX_FILES {
        out.push(reject(
            "",
            format!(
                "batch_too_large: at most {MAX_FILES} files, got {}",
                req.files.len()
            ),
        ));
    }
    let mut seen = BTreeSet::new();
    let mut incoming = 0usize;
    for file in &req.files {
        incoming = incoming.saturating_add(file.symbols.len());
        if !seen.insert(file.path.as_str()) {
            out.push(reject(
                &file.path,
                "duplicate_path: listed twice in one batch",
            ));
        }
        out.extend(validate_file(file));
    }
    for path in &req.removed {
        if !path_ok(path) {
            out.push(reject(path, "invalid_path: removed entry"));
        }
    }
    if let Some(pairs) = &req.cochange {
        if req.mined_commit.is_none() {
            out.push(reject(
                "",
                "cochange_without_cursor: mined_commit is required",
            ));
        }
        out.extend(pairs.iter().filter_map(validate_pair));
    }
    if existing_symbols.saturating_add(incoming) > SYMBOL_QUOTA {
        out.push(reject(
            "",
            format!("code_quota: repo would exceed {SYMBOL_QUOTA} symbols"),
        ));
    }
    out
}

fn validate_file(file: &CodeFileWire) -> Vec<CodeImportRejection> {
    let mut out = Vec::new();
    if !path_ok(&file.path) {
        out.push(reject(
            &file.path,
            "invalid_path: must be relative with no `..`",
        ));
    }
    if !blob_oid_ok(&file.blob_oid) {
        out.push(reject(
            &file.path,
            "invalid_blob_oid: expected a lowercase hex git object id (40 or 64 chars)",
        ));
    }
    let mut locations = BTreeSet::new();
    for s in &file.symbols {
        if s.symbol.is_empty() || s.kind.is_empty() || s.lang.is_empty() {
            out.push(reject(
                &file.path,
                "invalid_symbol: symbol, kind and lang are required",
            ));
        }
        if s.line_start < 1 || s.line_end < s.line_start {
            out.push(reject(
                &file.path,
                format!("invalid_symbol: bad line range for `{}`", s.symbol),
            ));
        }
        if !locations.insert((s.symbol.as_str(), s.line_start)) {
            out.push(reject(
                &file.path,
                format!("duplicate_symbol: `{}` at line {}", s.symbol, s.line_start),
            ));
        }
    }
    for target in &file.imports {
        if !path_ok(target) {
            out.push(reject(
                &file.path,
                format!("invalid_path: import target `{target}`"),
            ));
        }
    }
    out
}

fn validate_pair(pair: &CoChangeWire) -> Option<CodeImportRejection> {
    if !path_ok(&pair.from) || !path_ok(&pair.to) {
        return Some(reject(&pair.from, "invalid_path: cochange endpoint"));
    }
    if pair.from == pair.to {
        return Some(reject(&pair.from, "invalid_cochange: self pair"));
    }
    if pair.weight < 1 {
        return Some(reject(&pair.from, "invalid_cochange: weight must be >= 1"));
    }
    None
}
