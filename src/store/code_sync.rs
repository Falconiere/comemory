//! Reads and the one cursor behind the code-index push (`sync::code`): the
//! snippet-free per-file projection out of `code_symbols` / `edges`, the
//! repo's co-change set, and the `schema_meta` record of what was last
//! pushed. The server side of the same feature writes through `code_row`,
//! `indexed_files` and `edges` directly.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::prelude::*;
use crate::store::edges::{self, file_node_id, file_node_prefix};
use crate::store::schema_meta;

/// `schema_meta` key prefix of the per-repo pushed cursor.
const CURSOR_KEY_PREFIX: &str = "code_sync:";

/// One top-level symbol as the projection carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSymbolRow {
    /// Qualified symbol name.
    pub symbol: String,
    /// Symbol kind.
    pub kind: String,
    /// Language slug.
    pub lang: String,
    /// First line (1-based).
    pub line_start: i64,
    /// Last line (1-based, inclusive).
    pub line_end: i64,
}

/// What the last code push of `repo` left on the workspace, so a repeat
/// push can tell "nothing moved" without a network round trip.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeSyncCursor {
    /// `repo_marker.last_head` at the time of the push.
    pub pushed_head: Option<String>,
    /// `repo_marker.last_mined_commit` at the time of the push.
    pub pushed_mined_commit: Option<String>,
    /// SHA-256 over every `(path, blob_oid)` row at the time of the push, so
    /// a blob that moved without HEAD moving still reads as a change.
    #[serde(default)]
    pub pushed_digest: String,
    /// RFC3339 time of the push.
    pub pushed_at: String,
}

/// The file's top-level symbols (`parent_id IS NULL`), in source order —
/// chunk children share their parent's identity and are not carried.
pub fn parent_symbols_for_file(
    conn: &Connection,
    repo: &str,
    path: &str,
) -> Result<Vec<FileSymbolRow>> {
    let mut stmt = conn.prepare(
        "SELECT symbol, kind, lang, line_start, line_end FROM code_symbols \
          WHERE repo = ?1 AND path = ?2 AND parent_id IS NULL \
          ORDER BY line_start, symbol",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![repo, path], |r| {
            Ok(FileSymbolRow {
                symbol: r.get(0)?,
                kind: r.get(1)?,
                lang: r.get(2)?,
                line_start: r.get(3)?,
                line_end: r.get(4)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Repo-relative paths `path` imports, from its outgoing `imports` edges.
pub fn import_targets(conn: &Connection, repo: &str, path: &str) -> Result<Vec<String>> {
    let prefix = file_node_prefix(repo);
    let src = file_node_id(repo, path);
    let mut out: Vec<String> = edges::outgoing(conn, "file", &src, "imports")?
        .into_iter()
        .filter_map(|(_, dst)| dst.strip_prefix(&prefix).map(str::to_owned))
        .collect();
    out.sort();
    Ok(out)
}

/// Every stored `co_changed` pair of `repo` as `(from, to, weight)`, in the
/// canonical `(src, dst)` order the miner wrote.
pub fn co_changed_pairs(conn: &Connection, repo: &str) -> Result<Vec<(String, String, i64)>> {
    let prefix = file_node_prefix(repo);
    let rows = edges::co_changed_and_imports_edges(conn, &prefix)?;
    Ok(rows
        .into_iter()
        .filter(|row| row.rel == "co_changed")
        .filter_map(|row| {
            let from = row.src_id.strip_prefix(&prefix)?.to_owned();
            let to = row.dst_id.strip_prefix(&prefix)?.to_owned();
            Some((from, to, row.weight))
        })
        .collect())
}

/// `code_symbols` rows of `repo` on paths NOT in `paths` — what a batch
/// that replaces `paths` leaves untouched, so the import quota counts the
/// post-batch total rather than the same files twice. Chunked under
/// SQLite's bound-variable cap.
pub fn symbol_count_excluding(conn: &Connection, repo: &str, paths: &[&str]) -> Result<usize> {
    if paths.is_empty() {
        return crate::store::code_row::count_for_repo(conn, repo)
            .map(|n| usize::try_from(n).unwrap_or(0));
    }
    let total = crate::store::code_row::count_for_repo(conn, repo)?;
    let mut inside: i64 = 0;
    for chunk in paths.chunks(500) {
        let qmarks = crate::store::qmarks(chunk.len());
        let sql =
            format!("SELECT COUNT(*) FROM code_symbols WHERE repo = ?1 AND path IN ({qmarks})");
        let mut stmt = conn.prepare(&sql)?;
        let bound = std::iter::once(repo).chain(chunk.iter().copied());
        let n: i64 = stmt.query_row(rusqlite::params_from_iter(bound), |r| r.get(0))?;
        inside = inside.saturating_add(n);
    }
    Ok(usize::try_from(total.saturating_sub(inside)).unwrap_or(0))
}

/// The recorded cursor for `repo`, `None` when it was never pushed. A
/// value that no longer parses is treated as never pushed rather than
/// failing the push it gates — and logged, since the next push overwrites
/// it and an operator would otherwise never learn it was corrupt.
pub fn cursor(conn: &Connection, repo: &str) -> Result<Option<CodeSyncCursor>> {
    let Some(raw) = schema_meta::get(conn, &format!("{CURSOR_KEY_PREFIX}{repo}"))? else {
        return Ok(None);
    };
    match serde_json::from_str(&raw) {
        Ok(cursor) => Ok(Some(cursor)),
        Err(e) => {
            tracing::warn!(repo, error = %e, "code_sync cursor unreadable; treating as never pushed");
            Ok(None)
        }
    }
}

/// Record what was just pushed for `repo`.
pub fn set_cursor(conn: &Connection, repo: &str, cursor: &CodeSyncCursor) -> Result<()> {
    let json = serde_json::to_string(cursor)?;
    schema_meta::upsert(conn, &format!("{CURSOR_KEY_PREFIX}{repo}"), &json)
}

#[cfg(test)]
#[path = "tests/code_sync.rs"]
mod tests;
