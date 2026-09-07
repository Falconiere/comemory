//! `repo_marker.last_mined_commit` — the co-change mining cursor read and
//! advanced by [`crate::graph::materialize`]. Kept separate from
//! `code_row.rs` (which owns `root_path`/`last_head`/`last_indexed_at`, the
//! `index-code` writer's own fields) and `repo_marker_roots.rs` (the serve
//! layer's root reads): this column has its own writer and its own caller.

use rusqlite::{Connection, OptionalExtension, params};

use crate::prelude::*;

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
