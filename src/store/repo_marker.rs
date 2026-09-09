//! `repo_marker.last_mined_commit` — the co-change mining cursor read and
//! advanced by [`crate::graph::materialize`]. Kept separate from
//! `code_row.rs` (which owns `root_path`/`last_head`/`last_indexed_at`, the
//! `index-code` writer's own fields) and `repo_marker_roots.rs` (the serve
//! layer's root reads): this column has its own writer and its own caller.
//!
//! [`read_for_lazy_reindex`] is the one exception to the single-column split:
//! `cli::lazy_reindex`'s staleness probe needs `last_mined_commit`,
//! `root_path`, and `archived` in the same round trip, so it stays one query
//! rather than three.

use rusqlite::{Connection, OptionalExtension, params};

use crate::prelude::*;

/// The `repo_marker` columns [`crate::cli::lazy_reindex::maybe_trigger`]'s
/// staleness probe needs in one row: the HEAD at last index
/// (`last_mined_commit`), the absolute working-tree root captured at index
/// time (`root_path`, `NULL` for never-indexed / pre-v7 repos), and the
/// `archived` flag (v15) that suppresses the trigger entirely.
pub(crate) struct LazyReindexMarker {
    /// HEAD oid at the last successful mine, or `None` if never mined.
    pub last_mined_commit: Option<String>,
    /// Absolute working-tree root captured at index time.
    pub root_path: Option<String>,
    /// Whether the repo is archived (no longer auto-reindexed).
    pub archived: bool,
}

/// `SELECT <column> FROM repo_marker WHERE repo = ?1`, as `Ok(None)` when the
/// repo has no marker row at all.
///
/// `column` is `&'static str` and is interpolated into the SQL, so only a
/// compile-time literal at a call site can reach it — `repo`, the one
/// genuinely dynamic value, is bound. The three single-column readers below
/// differ only in that literal and in how they interpret the value, which is
/// why they share this body rather than repeating the query three times.
fn column_for_repo<T: rusqlite::types::FromSql>(
    conn: &Connection,
    column: &'static str,
    repo: &str,
) -> Result<Option<T>> {
    let sql = format!("SELECT {column} FROM repo_marker WHERE repo = ?1");
    conn.query_row(&sql, [repo], |r| r.get::<_, T>(0))
        .optional()
        .map_err(Error::Sqlite)
}

/// The `repo_marker` row for `repo`, or `None` when there is no marker row
/// (never indexed).
pub(crate) fn read_for_lazy_reindex(
    conn: &Connection,
    repo: &str,
) -> Result<Option<LazyReindexMarker>> {
    conn.query_row(
        "SELECT last_mined_commit, root_path, archived FROM repo_marker WHERE repo = ?1",
        [repo],
        |r| {
            Ok(LazyReindexMarker {
                last_mined_commit: r.get::<_, Option<String>>(0)?,
                root_path: r.get::<_, Option<String>>(1)?,
                archived: r.get::<_, i64>(2)? != 0,
            })
        },
    )
    .optional()
    .map_err(Error::Sqlite)
}

/// The stored `last_mined_commit` cursor for `repo`, or `None` when the repo
/// has never been mined, or its marker row's cursor column is `NULL`.
pub(crate) fn last_mined_commit(conn: &Connection, repo: &str) -> Result<Option<String>> {
    Ok(column_for_repo::<Option<String>>(conn, "last_mined_commit", repo)?.flatten())
}

/// Advance `repo_marker.last_mined_commit` to `cursor`, creating the marker
/// row on first mine. Other `repo_marker` columns are preserved by the
/// targeted `UPDATE`.
pub(crate) fn advance_mined_cursor(conn: &Connection, repo: &str, cursor: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO repo_marker(repo, last_mined_commit) VALUES(?1, ?2) \
         ON CONFLICT(repo) DO UPDATE SET last_mined_commit = excluded.last_mined_commit",
        params![repo, cursor],
    )?;
    Ok(())
}

/// Whether `repo`'s `repo_marker.archived` flag is set. `None` when the
/// repo has no marker row yet (never indexed) — `api::index_code`'s
/// archived-repo refusal treats an unknown repo as not archived.
pub fn archived(conn: &Connection, repo: &str) -> Result<Option<bool>> {
    Ok(column_for_repo::<i64>(conn, "archived", repo)?.map(|f| f != 0))
}

/// Every `repo_marker` label, ascending — the authoritative repo list
/// behind `api::graph_recompute`'s "rescore every repo" walk.
pub(crate) fn all_repos(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT repo FROM repo_marker ORDER BY repo")?;
    let rows = stmt
        .query_map([], |r| r.get(0))?
        .collect::<std::result::Result<Vec<String>, _>>()?;
    Ok(rows)
}

/// `repo_marker.root_path` for `repo`, or `None` when there is no marker
/// row (or its root is `NULL`) — behind `api::repo_admin`'s connect/patch.
pub(crate) fn root_path(conn: &Connection, repo: &str) -> Result<Option<String>> {
    Ok(column_for_repo::<Option<String>>(conn, "root_path", repo)?.flatten())
}

/// Whether a `repo_marker` row exists for `repo` — behind
/// `api::repo_admin`'s "unknown repo" `404` gate.
pub(crate) fn exists(conn: &Connection, repo: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM repo_marker WHERE repo = ?1)",
        [repo],
        |r| r.get(0),
    )
    .map_err(Error::from)
}

/// Set `repo_marker.archived` for `repo`; the returned row count is `0`
/// when the label is unknown — `api::repo_admin::archive`'s `404` gate.
pub(crate) fn set_archived(conn: &Connection, repo: &str, archived: bool) -> Result<usize> {
    conn.execute(
        "UPDATE repo_marker SET archived = ?2 WHERE repo = ?1",
        params![repo, i64::from(archived)],
    )
    .map_err(Error::from)
}

#[cfg(test)]
#[path = "tests/repo_marker.rs"]
mod tests;
