//! Resolve a `file:<repo>:<path>` graph node id to an absolute file on disk.
//!
//! comemory stores only a repo label + repo-relative paths, plus the absolute
//! working-tree root captured at index time in `repo_marker.root_path` (v7).
//! This module turns a node id back into a real path: pick the root (a
//! `--root` override wins over the stored value, which wins over an error),
//! then hand off to [`crate::utilities::path_containment::resolve_within`] for the
//! canonicalize-and-contain check. Every file read and write funnels through
//! [`id_to_abs_path`], so the containment guarantee has a single chokepoint.
//!
//! [`parse_id`] — the decoder for that id grammar, and the inverse of
//! `store::edges::file_node_id` — lives here rather than in a delivery module,
//! where it sat until #167: a shared primitive may not reach into a delivery
//! adapter, and every consumer of the resolver needs the decoder too.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::prelude::*;
use crate::store::Connection;
use crate::store::repo_marker_roots;
use crate::utilities::path_containment;

/// `--root <repo>=<path>` overrides, keyed by repo label.
pub type RootOverrides = HashMap<String, PathBuf>;

/// Split a canonical file node id (`file:<repo>:<path>`) into `(repo, path)`.
/// Returns `None` for ids that do not follow the convention. Assumes repo
/// labels contain no `:` (the same assumption baked into
/// `store::edges::file_node_prefix`'s `substr` predicate); a repo with a `:`
/// would split on the wrong colon.
pub fn parse_id(id: &str) -> Option<(&str, &str)> {
    id.strip_prefix("file:")?.split_once(':')
}

/// Resolve the canonical absolute working-tree root for `repo`. Precedence:
/// an explicit `--root` override, then `repo_marker.root_path`, then an error
/// instructing the caller to supply `--root`. The chosen root is canonicalized
/// so the containment check in [`path_containment::resolve_within`] has a stable,
/// symlink-resolved prefix to compare against.
pub fn resolve_root(conn: &Connection, repo: &str, overrides: &RootOverrides) -> Result<PathBuf> {
    if let Some(p) = overrides.get(repo) {
        return p
            .canonicalize()
            .map_err(|e| Error::BadRequest(format!("--root for `{repo}` is unusable: {e}")));
    }
    // Only "no such repo row" means "no stored root"; a real query error
    // (e.g. a half-applied v7 migration with no `root_path` column) must not be
    // disguised as the friendly "pass --root" hint.
    match repo_marker_roots::root_path(conn, repo)? {
        Some(path) => PathBuf::from(&path)
            .canonicalize()
            .map_err(|e| Error::BadRequest(format!("stored root for `{repo}` is unusable: {e}"))),
        None => Err(Error::BadRequest(format!(
            "repo root unknown for `{repo}`; pass `--root {repo}=<path>`"
        ))),
    }
}

/// Turn a `file:<repo>:<path>` node id into an absolute, contained path on
/// disk. Returns `BadRequest` for ids that do not follow the convention and
/// `Forbidden` for paths that escape the resolved repo root.
pub fn id_to_abs_path(conn: &Connection, id: &str, overrides: &RootOverrides) -> Result<PathBuf> {
    let (repo, rel) =
        parse_id(id).ok_or_else(|| Error::BadRequest(format!("invalid file node id: {id}")))?;
    let root = resolve_root(conn, repo, overrides)?;
    path_containment::resolve_within(&root, rel)
}

/// Split the `path` (repo-relative) out of a `file:<repo>:<path>` id for
/// display. Returns `None` for malformed ids.
pub fn rel_of(id: &str) -> Option<&str> {
    parse_id(id).map(|(_, rel)| rel)
}

#[cfg(test)]
#[path = "tests/repo_root.rs"]
mod tests;
