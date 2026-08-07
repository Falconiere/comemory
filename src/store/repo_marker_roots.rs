//! Enumerate every distinct working-tree root recorded in `repo_marker`, for
//! `comemory serve`'s allowed-roots set (`serve::security::contain_abs`
//! containment). Kept separate from `code_row.rs` (which owns the
//! per-repo *writer*, `upsert_repo_root`) because this is a read-side
//! enumerator with a different caller (the serve layer, not the indexer).

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
    let mut stmt = conn.prepare("SELECT root_path FROM repo_marker WHERE root_path IS NOT NULL")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut roots = Vec::new();
    for row in rows {
        let raw = row?;
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

#[cfg(test)]
#[path = "tests/repo_marker_roots.rs"]
mod tests;
