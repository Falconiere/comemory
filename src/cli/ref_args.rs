//! Parse the `comemory save` `--ref-file` / `--ref-symbol` flags into a
//! [`References`] block: qualify the repo, rewrite the path to repo-root-
//! relative form, and capture a committed-state anchor (HEAD blob + commit +
//! branch) for tracked refs in the cwd repo. Untracked / cross-repo refs stay
//! unpinned (+ advisory warning); a malformed value is `EX_USAGE` (exit 64).

use std::path::Path;

use crate::git_utils;
use crate::memory::{Ref, References};
use crate::prelude::*;

/// A reference's resolved repo plus the repo-root-relative path used both as
/// the `dst_id` component and as the key for the git blob lookup.
struct Qualified {
    /// Resolved repo (explicit `<repo>:` prefix, else the save repo).
    repo: String,
    /// Repo-root-relative path.
    path: String,
    /// Symbol name, present only for `--ref-symbol` values.
    symbol: Option<String>,
}

/// Build the [`References`] block (with captured anchors) plus advisory
/// warning strings from the raw `--ref-file` / `--ref-symbol` flag values.
///
/// `repo` is the save's already-resolved repo (used to qualify unprefixed
/// values and to decide whether a ref is anchorable). `repo_root`, when
/// `Some`, is the discovered git working-tree root; paths are made relative
/// to it and anchors are captured against its HEAD tree.
///
/// # Errors
/// Returns a usage error (`EX_USAGE`, exit 64) naming the offending value
/// when a ref is empty or a `--ref-symbol` value lacks a `:symbol` segment.
pub fn collect(
    ref_file: &[String],
    ref_symbol: &[String],
    repo: &str,
    repo_root: Option<&Path>,
) -> Result<(References, Vec<String>)> {
    let mut refs = References::default();
    let mut warnings = Vec::new();
    for raw in flatten(ref_file) {
        let q = qualify(&raw, repo, repo_root, false)?;
        refs.files
            .push(capture_anchor(q, repo, repo_root, &mut warnings)?);
    }
    for raw in flatten(ref_symbol) {
        let q = qualify(&raw, repo, repo_root, true)?;
        refs.symbols
            .push(capture_anchor(q, repo, repo_root, &mut warnings)?);
    }
    Ok((refs, warnings))
}

/// Flatten repeatable occurrences and comma-split each one, trimming and
/// dropping empty fragments. Empty *whole* values are reported by [`qualify`].
fn flatten(values: &[String]) -> Vec<String> {
    values
        .iter()
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Split a raw value into repo / repo-root-relative path / optional symbol.
///
/// `--ref-file`: `[repo:]path` (1 seg = path; 2 = repo, path).
/// `--ref-symbol`: `[repo:]path:symbol` (2 = path, symbol; 3 = repo, path,
/// symbol). Fewer segments than required, or any empty segment, is a usage
/// error naming the value.
fn qualify(raw: &str, repo: &str, repo_root: Option<&Path>, is_symbol: bool) -> Result<Qualified> {
    let segs: Vec<&str> = raw.split(':').collect();
    if segs.iter().any(|s| s.trim().is_empty()) {
        return Err(usage(raw, is_symbol));
    }
    let (repo_part, path_part, symbol) = match (is_symbol, segs.as_slice()) {
        (false, [p]) => (repo.to_string(), (*p).to_string(), None),
        (false, [r, p]) => ((*r).to_string(), (*p).to_string(), None),
        (true, [p, s]) => (repo.to_string(), (*p).to_string(), Some((*s).to_string())),
        (true, [r, p, s]) => ((*r).to_string(), (*p).to_string(), Some((*s).to_string())),
        _ => return Err(usage(raw, is_symbol)),
    };
    Ok(Qualified {
        repo: repo_part,
        path: normalize_path(&path_part, repo_root),
        symbol,
    })
}

/// Make `path` (cwd-relative) repo-root-relative by a lexical join + strip —
/// never canonicalizing, so a not-yet-created file still normalizes. Falls
/// back to the given path when no repo root is known or the strip fails.
fn normalize_path(path: &str, repo_root: Option<&Path>) -> String {
    let Some(root) = repo_root else {
        return path.to_string();
    };
    let Ok(cwd) = std::env::current_dir() else {
        return path.to_string();
    };
    cwd.join(path)
        .strip_prefix(root)
        .ok()
        .and_then(Path::to_str)
        .map(str::to_string)
        .unwrap_or_else(|| path.to_string())
}

/// Capture the committed-state anchor for an anchorable ref, or return an
/// unpinned [`Ref`] and push an advisory warning. Anchorable = the ref's repo
/// equals the save repo, a repo root is known, and the path is tracked in the
/// HEAD tree (a dirty working copy still pins the last committed blob).
///
/// # Errors
/// Propagates a hard git fault from the HEAD queries
/// ([`git_utils::blob_oid_at_head`] and, for a tracked blob, the commit /
/// branch lookups): a corrupt or unreadable repository must surface, not be
/// silently saved unpinned. A legitimately untracked path (`Ok(None)`) is
/// *not* an error — it yields an unpinned [`Ref`] plus an advisory warning.
fn capture_anchor(
    q: Qualified,
    repo: &str,
    repo_root: Option<&Path>,
    warnings: &mut Vec<String>,
) -> Result<Ref> {
    let id = build_id(&q);
    let Some(root) = repo_root.filter(|_| q.repo == repo) else {
        warnings.push(format!(
            "{id} references a repo not on disk — saved unpinned"
        ));
        return Ok(Ref::new(id));
    };
    match git_utils::blob_oid_at_head(root, &q.path)? {
        Some(blob) => Ok(Ref {
            commit: Some(git_utils::current_head(root)?),
            branch: git_utils::current_branch(root)?,
            blob: Some(blob),
            id,
        }),
        None => {
            warnings.push(format!("{id} is untracked or missing — saved unpinned"));
            Ok(Ref::new(id))
        }
    }
}

/// Compose the qualified `dst_id`: `<repo>:<path>` or `<repo>:<path>:<symbol>`.
fn build_id(q: &Qualified) -> String {
    match &q.symbol {
        Some(sym) => format!("{}:{}:{}", q.repo, q.path, sym),
        None => format!("{}:{}", q.repo, q.path),
    }
}

/// Usage error (`EX_USAGE`, exit 64 via [`Error::Usage`]) naming the bad
/// value. `--ref-symbol` requires a trailing `:symbol`; both flags reject
/// empty / whitespace segments.
fn usage(raw: &str, is_symbol: bool) -> Error {
    let flag = if is_symbol {
        "--ref-symbol"
    } else {
        "--ref-file"
    };
    let detail = if is_symbol {
        " (expected `[repo:]path:symbol`)"
    } else {
        " (expected `[repo:]path`)"
    };
    Error::Usage(format!("{flag}: malformed reference `{raw}`{detail}"))
}
