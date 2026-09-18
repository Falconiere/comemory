//! Reads over `repo_marker.root_path`, for `comemory serve`: [`all_roots`]
//! enumerates every distinct working-tree root (the allowed-roots set behind
//! `utilities::path_containment::contain_abs` containment), and [`root_path`] looks up
//! one repo's stored root (behind `utilities::repo_root::resolve_root`'s
//! `--root`-override fallback). Kept separate from `code_row.rs` (which owns
//! the per-repo *writer*, `upsert_repo_root`) because these are read-side
//! queries with a different caller (the serve layer, not the indexer).

use super::{
    orm,
    schema_code::{RepoMarker, repo_marker as c},
};
use toolu_orm::core::query_column::CommonOps;

use std::path::PathBuf;

use rusqlite::Connection;

use crate::prelude::*;

/// Every distinct, canonicalized `repo_marker.root_path` (`NULL` rows
/// skipped). A row whose stored path no longer canonicalizes — e.g. the
/// directory was deleted since indexing — is skipped with a warning rather
/// than failing the whole enumeration: this is a best-effort enrichment of
/// the allowed-roots set, not a hard requirement (mirrors the skip-and-warn
/// pattern in `graph::pagerank`/`graph::derived`).
pub fn all_roots(conn: &Connection) -> Result<Vec<PathBuf>> {
    let rows = orm::query_all(
        conn,
        RepoMarker::select()
            .columns_typed(&[&c::root_path])
            .filter(c::root_path.is_not_null())
            .to_sql(),
        |r| r.get::<_, String>(0),
    )?;
    let mut roots = Vec::new();
    for raw in rows {
        match PathBuf::from(&raw).canonicalize() {
            Ok(canonical) => roots.push(canonical),
            Err(e) => {
                tracing::warn!(
                    root_path = raw,
                    error = %e,
                    "repo_marker_roots: skipping unusable stored root"
                );
            }
        }
    }
    Ok(roots)
}

/// Stored `repo_marker.root_path` for `repo`. `Ok(None)` when no row exists
/// for `repo` OR the stored `root_path` is `NULL` (a repo indexed before v7,
/// no `--root` ever supplied) — both are "no stored root", the caller's cue
/// to fall back to a friendly `--root` hint. A genuine query failure (e.g. a
/// half-applied v7 migration missing the `root_path` column) propagates as
/// `Err` instead, so it is never disguised as that same hint.
pub fn root_path(conn: &Connection, repo: &str) -> Result<Option<String>> {
    Ok(orm::query_optional(
        conn,
        RepoMarker::select()
            .columns_typed(&[&c::root_path])
            .filter(c::repo.eq(repo))
            .to_sql(),
        |r| r.get::<_, Option<String>>(0),
    )?
    .flatten())
}

#[cfg(test)]
#[path = "tests/repo_marker_roots.rs"]
mod tests;
