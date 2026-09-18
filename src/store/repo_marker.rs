//! `repo_marker.last_mined_commit` — the co-change mining cursor read and
//! advanced by [`crate::domains::graph::materialize`]. Kept separate from
//! `code_row.rs` (which owns `root_path`/`last_head`/`last_indexed_at`, the
//! `index-code` writer's own fields) and `repo_marker_roots.rs` (the serve
//! layer's root reads): this column has its own writer and its own caller.
//!
//! [`read_for_lazy_reindex`] is the one exception to the single-column split:
//! `cli::lazy_reindex`'s staleness probe needs `last_mined_commit`,
//! `root_path`, and `archived` in the same round trip, so it stays one query
//! rather than three.

use super::{
    orm,
    schema_code::{RepoMarker, repo_marker as c},
};
use rusqlite::{Connection, params};
use toolu_orm::core::query_column::{ColumnRef, CommonOps};

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

/// Read one declared marker column, retaining an absent marker as `None`.
fn column_for_repo<T: rusqlite::types::FromSql>(
    conn: &Connection,
    column: &dyn ColumnRef,
    repo: &str,
) -> Result<Option<T>> {
    orm::query_optional(
        conn,
        RepoMarker::select()
            .columns_typed(&[column])
            .filter(c::repo.eq(repo))
            .to_sql(),
        |r| r.get(0),
    )
}

/// The `repo_marker` row for `repo`, or `None` when there is no marker row
/// (never indexed).
pub(crate) fn read_for_lazy_reindex(
    conn: &Connection,
    repo: &str,
) -> Result<Option<LazyReindexMarker>> {
    orm::query_optional(
        conn,
        RepoMarker::select()
            .columns_typed(&[&c::last_mined_commit, &c::root_path, &c::archived])
            .filter(c::repo.eq(repo))
            .to_sql(),
        |r| {
            Ok(LazyReindexMarker {
                last_mined_commit: r.get(0)?,
                root_path: r.get(1)?,
                archived: r.get::<_, i64>(2)? != 0,
            })
        },
    )
}

/// The stored `last_mined_commit` cursor for `repo`, or `None` when the repo
/// has never been mined, or its marker row's cursor column is `NULL`.
pub(crate) fn last_mined_commit(conn: &Connection, repo: &str) -> Result<Option<String>> {
    Ok(column_for_repo::<Option<String>>(conn, &c::last_mined_commit, repo)?.flatten())
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

/// The stored `last_head` for `repo`, or `None` when the repo has never
/// been indexed (or the column is `NULL`) — the head a code push reports
/// and a code manifest answers.
pub(crate) fn last_head(conn: &Connection, repo: &str) -> Result<Option<String>> {
    Ok(column_for_repo::<Option<String>>(conn, &c::last_head, repo)?.flatten())
}

/// Whether `repo`'s `repo_marker.archived` flag is set. `None` when the
/// repo has no marker row yet (never indexed) — `domains::code::index_code`'s
/// archived-repo refusal treats an unknown repo as not archived.
pub fn archived(conn: &Connection, repo: &str) -> Result<Option<bool>> {
    Ok(column_for_repo::<i64>(conn, &c::archived, repo)?.map(|f| f != 0))
}

/// Every `repo_marker` label, ascending — the authoritative repo list
/// behind `domains::graph::graph_recompute`'s "rescore every repo" walk.
pub(crate) fn all_repos(conn: &Connection) -> Result<Vec<String>> {
    orm::query_all(
        conn,
        RepoMarker::select()
            .columns_typed(&[&c::repo])
            .order_by(c::repo.asc())
            .to_sql(),
        |r| r.get(0),
    )
}

/// `repo_marker.root_path` for `repo`, or `None` when there is no marker
/// row (or its root is `NULL`) — behind `domains::code::repo_admin`'s connect/patch.
pub(crate) fn root_path(conn: &Connection, repo: &str) -> Result<Option<String>> {
    Ok(column_for_repo::<Option<String>>(conn, &c::root_path, repo)?.flatten())
}

/// Whether a `repo_marker` row exists for `repo` — behind
/// `domains::code::repo_admin`'s "unknown repo" `404` gate.
pub(crate) fn exists(conn: &Connection, repo: &str) -> Result<bool> {
    orm::query_one(
        conn,
        RepoMarker::select()
            .filter(c::repo.eq(repo))
            .to_exists_sql(),
        |r| r.get(0),
    )
}

/// Set `repo_marker.archived` for `repo`; the returned row count is `0`
/// when the label is unknown — `crate::domains::code::repo_admin::archive`'s `404` gate.
pub(crate) fn set_archived(conn: &Connection, repo: &str, archived: bool) -> Result<usize> {
    orm::execute(
        conn,
        RepoMarker::update()
            .set(&c::archived, i64::from(archived))
            .filter(c::repo.eq(repo))
            .to_sql(),
    )
}

#[cfg(test)]
#[path = "tests/repo_marker.rs"]
mod tests;
