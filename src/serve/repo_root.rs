//! Resolve a `file:<repo>:<path>` graph node id to an absolute file on disk.
//!
//! comemory stores only a repo label + repo-relative paths, plus the absolute
//! working-tree root captured at index time in `repo_marker.root_path` (v7).
//! This module turns a node id back into a real path: pick the root (a
//! `--root` override wins over the stored value, which wins over an error),
//! then hand off to [`crate::serve::security::resolve_within`] for the
//! canonicalize-and-contain check. Every file read and write funnels through
//! [`id_to_abs_path`], so the containment guarantee has a single chokepoint.

use std::collections::HashMap;
use std::path::PathBuf;

use rusqlite::Connection;

use crate::cli::graph::parse_id;
use crate::prelude::*;
use crate::serve::security;

/// `--root <repo>=<path>` overrides, keyed by repo label.
pub type RootOverrides = HashMap<String, PathBuf>;

/// Resolve the canonical absolute working-tree root for `repo`. Precedence:
/// an explicit `--root` override, then `repo_marker.root_path`, then an error
/// instructing the caller to supply `--root`. The chosen root is canonicalized
/// so the containment check in [`security::resolve_within`] has a stable,
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
    let stored: Option<String> = match conn.query_row(
        "SELECT root_path FROM repo_marker WHERE repo = ?1",
        [repo],
        |r| r.get::<_, Option<String>>(0),
    ) {
        Ok(v) => v,
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(e) => return Err(Error::Sqlite(e)),
    };
    match stored {
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
    security::resolve_within(&root, rel)
}

/// Split the `path` (repo-relative) out of a `file:<repo>:<path>` id for
/// display. Returns `None` for malformed ids.
pub fn rel_of(id: &str) -> Option<&str> {
    parse_id(id).map(|(_, rel)| rel)
}
