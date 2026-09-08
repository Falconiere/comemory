//! `indexed_files` cursor reads not already owned by `code_row.rs`'s
//! upsert. Table CRUD is split across the two files rather than
//! consolidated: `upsert_indexed_file` lives in `code_row.rs` because it is
//! called from the same per-file loop as the `code_symbols` insert it
//! follows, and moving it here would gain nothing but an extra module
//! boundary between two calls that always run together.

use rusqlite::{Connection, OptionalExtension, params};

use crate::prelude::*;

/// Drop every `indexed_files` cursor row for `repo`, forcing the next
/// `index-code` walk to re-extract every file. Shared by
/// [`crate::store::code_row::ensure_repo_format`] (the code-format-version
/// gate) and `api::index_code::run_with_progress`'s `--mode full`.
pub fn delete_for_repo(conn: &Connection, repo: &str) -> Result<()> {
    conn.execute("DELETE FROM indexed_files WHERE repo = ?1", [repo])?;
    Ok(())
}

/// The recorded blob OID for `(repo, path)`, or `None` when the file has
/// never been indexed. Behind `api::index_code::walk`'s incremental skip
/// gate — the caller compares this against the file's current blob OID.
pub fn blob_oid_for(conn: &Connection, repo: &str, path: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT blob_oid FROM indexed_files WHERE repo = ?1 AND path = ?2",
        params![repo, path],
        |r| r.get(0),
    )
    .optional()
    .map_err(Error::from)
}

#[cfg(test)]
#[path = "tests/indexed_files.rs"]
mod tests;
