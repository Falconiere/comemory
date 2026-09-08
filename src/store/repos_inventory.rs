//! The `repo_marker` join behind `comemory repos` / `GET /api/v1/repos`:
//! one row per indexed repo, joined against its per-repo file/symbol/memory
//! counters. Git-state resolution and JSON shaping stay in `api::repos`
//! (spec `docs/toolu/specs/2026-09-07-store-layer-chokepoint-design.md`).

use rusqlite::Connection;

use crate::prelude::*;

/// One `repo_marker` row plus its joined counters, before git resolution.
pub struct RepoMarkerRow {
    /// The repo label.
    pub repo: String,
    /// Absolute working-tree root captured at index time.
    pub root_path: Option<String>,
    /// HEAD oid recorded by the last successful `index-code` run.
    pub last_head: Option<String>,
    /// Timestamp of the last successful `index-code` run.
    pub last_indexed_at: Option<String>,
    /// Distinct paths in `indexed_files` for this repo.
    pub files: u64,
    /// Rows in `code_symbols` for this repo, including cAST chunk children.
    pub symbols: u64,
    /// Live memories (`deleted_at IS NULL`) filed under this repo label.
    pub memories: u64,
    /// `repo_marker.archived`.
    pub archived: bool,
}

/// Join `repo_marker` against the per-repo counters, narrowed to `repo`
/// when one was requested, ordered by repo label for deterministic output.
///
/// Every counter subquery carries an alias and the mapper reads BY NAME:
/// adding a column, or moving one, then cannot silently shift what each
/// field is filled from.
pub fn fetch(conn: &Connection, repo: Option<&str>) -> Result<Vec<RepoMarkerRow>> {
    let sql = "SELECT rm.repo, rm.root_path, rm.last_head, rm.last_indexed_at, rm.archived, \
                      (SELECT COUNT(DISTINCT path) FROM indexed_files WHERE repo = rm.repo) AS files, \
                      (SELECT COUNT(*) FROM code_symbols WHERE repo = rm.repo) AS symbols, \
                      (SELECT COUNT(*) FROM memories WHERE repo = rm.repo AND deleted_at IS NULL) \
                        AS memories \
               FROM repo_marker rm \
               WHERE (?1 IS NULL OR rm.repo = ?1) \
               ORDER BY rm.repo";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([repo], |r| {
        Ok(RepoMarkerRow {
            repo: r.get("repo")?,
            root_path: r.get("root_path")?,
            last_head: r.get("last_head")?,
            last_indexed_at: r.get("last_indexed_at")?,
            archived: r.get::<_, i64>("archived")? != 0,
            files: r.get::<_, i64>("files")? as u64,
            symbols: r.get::<_, i64>("symbols")? as u64,
            memories: r.get::<_, i64>("memories")? as u64,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

#[cfg(test)]
#[path = "tests/repos_inventory.rs"]
mod tests;
