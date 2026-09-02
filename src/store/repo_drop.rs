//! `store::repo_drop` — drop every code-index row and edge for one repo
//! label, keeping its memories (`DELETE /api/v1/repos/{name}`).
//!
//! One transaction over the six code-side tables plus the repo's file
//! nodes in `edges`, so a failure part-way leaves the repo fully indexed
//! rather than half-dropped. `code_fts` and `code_vec` are virtual tables
//! with no foreign key onto `code_symbols`, so their rows are deleted
//! explicitly first (exactly as `code_row::purge_file_symbols` does for one
//! file), while `code_symbols` rows are still there to name them.
//!
//! What is deliberately KEPT (spec §10, AC-18): every `memories` row filed
//! under the label, and the memory→code reference edges those memories own
//! (`references_file` / `references_symbol`, whose `dst_id` is the bare
//! `<repo>:<path>[:<symbol>]` form, not a `file:` node). Disconnecting a
//! repo drops its code index; it does not delete what was written about it.
//! Re-indexing the same label re-materializes the code rows those edges
//! point at.
//!
//! Disconnecting does NOT stop future indexing: the `repo_marker` row goes
//! too, so under the default `COMEMORY_INDEXING_AUTO_REINDEX=lazy` the next
//! `search-code` / `context` run from that checkout sees a never-indexed
//! repo and spawns a background `index-code` that rebuilds the index (only
//! `hook` / `off` leave it alone). To stop indexing while keeping the
//! memories, archive the repo instead (`api::repo_admin::archive`), which
//! lazy reindex skips.

use rusqlite::Connection;
use serde::Serialize;

use crate::graph::{derived, edges};
use crate::prelude::*;

/// What [`drop_repo`] removed, per table group — the `DELETE
/// /api/v1/repos/{name}` response body's counters.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct DropCounts {
    /// `code_symbols` rows removed (cAST chunk children included).
    pub symbols_removed: u64,
    /// `indexed_files` cursor rows removed.
    pub files_removed: u64,
    /// `edges` rows removed — those with a `file:<repo>:<path>` node on
    /// either side.
    pub edges_removed: u64,
}

/// Delete every code-index row and file-node edge for `repo` in ONE
/// transaction, then best-effort refresh the derived artifacts
/// (`memories.rank_score`, `edge_fts`) the removed edges fed.
///
/// Dropping a label that was never indexed is not an error: every statement
/// simply matches zero rows and the counters come back zero. Callers that
/// need "unknown repo" to be a `404` check `repo_marker` first
/// (`api::repo_admin::disconnect`).
pub fn drop_repo(conn: &mut Connection, repo: &str) -> Result<DropCounts> {
    let prefix = edges::file_node_prefix(repo);
    let prefix_len = i64::try_from(prefix.chars().count()).unwrap_or(i64::MAX);
    let tx = conn.transaction()?;

    // Virtual tables first: both are keyed by `code_symbols.id`, which the
    // subselect can only resolve while those rows still exist.
    tx.execute(
        "DELETE FROM code_vec WHERE symbol_id IN (SELECT id FROM code_symbols WHERE repo = ?1)",
        [repo],
    )?;
    tx.execute(
        "DELETE FROM code_fts WHERE symbol_id IN (SELECT id FROM code_symbols WHERE repo = ?1)",
        [repo],
    )?;
    // `code_feedback` is keyed by (repo, path, symbol), not by symbol id.
    tx.execute("DELETE FROM code_feedback WHERE repo = ?1", [repo])?;
    let symbols_removed = tx.execute("DELETE FROM code_symbols WHERE repo = ?1", [repo])?;
    let files_removed = tx.execute("DELETE FROM indexed_files WHERE repo = ?1", [repo])?;
    // Prefix match by `substr`, never `LIKE`: a repo label containing `%`
    // or `_` would otherwise match foreign nodes.
    let edges_removed = tx.execute(
        "DELETE FROM edges \
          WHERE (src_kind = 'file' AND substr(src_id, 1, ?2) = ?1) \
             OR (dst_kind = 'file' AND substr(dst_id, 1, ?2) = ?1)",
        rusqlite::params![prefix, prefix_len],
    )?;
    tx.execute("DELETE FROM repo_marker WHERE repo = ?1", [repo])?;
    tx.commit()?;

    let _stale = derived::refresh_derived_best_effort(conn);
    Ok(DropCounts {
        symbols_removed: count(symbols_removed),
        files_removed: count(files_removed),
        edges_removed: count(edges_removed),
    })
}

/// A rusqlite affected-row count as the `u64` the response carries.
fn count(rows: usize) -> u64 {
    u64::try_from(rows).unwrap_or(u64::MAX)
}

#[cfg(test)]
#[path = "tests/repo_drop.rs"]
mod tests;
