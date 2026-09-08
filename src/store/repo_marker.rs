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
    let cursor = conn
        .query_row(
            "SELECT last_mined_commit FROM repo_marker WHERE repo = ?1",
            [repo],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    Ok(cursor)
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

#[cfg(test)]
#[path = "tests/repo_marker.rs"]
mod tests;
