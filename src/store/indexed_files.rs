//! `indexed_files` cursor reads not already owned by `code_row.rs`'s
//! upsert. Table CRUD is split across the two files rather than
//! consolidated: `upsert_indexed_file` lives in `code_row.rs` because it is
//! called from the same per-file loop as the `code_symbols` insert it
//! follows, and moving it here would gain nothing but an extra module
//! boundary between two calls that always run together.

use super::{
    orm,
    schema_code::{IndexedFiles, indexed_files as c},
};
use toolu_orm::core::query_column::{ColumnRef, CommonOps};
use toolu_orm::query::select::SelectBuilder;

use rusqlite::Connection;

use crate::prelude::*;

orm::writes! {
    /// Drop every `indexed_files` cursor row for `repo`, forcing the next
    /// `index-code` walk to re-extract every file. Shared by
    /// [`crate::store::code_row::ensure_repo_format`] (the code-format-version
    /// gate) and `crate::domains::code::index_code::run_with_progress`'s `--mode full`.
    pub delete_for_repo(repo: &str) = IndexedFiles::delete().filter(c::repo.eq(repo));
    /// Drop the cursor row for one `(repo, path)` — the removal half of a code
    /// import; a path with no row is a no-op.
    pub delete_one(repo: &str, path: &str) =
        IndexedFiles::delete().filter(c::repo.eq(repo)).filter(c::path.eq(path));
}

/// A `SELECT` of `columns` over the cursor rows of one `repo`, ready for the
/// caller to narrow or order.
fn repo_rows(columns: &[&dyn ColumnRef], repo: &str) -> SelectBuilder {
    IndexedFiles::select()
        .columns_typed(columns)
        .filter(c::repo.eq(repo))
}

/// The recorded blob OID for `(repo, path)`, or `None` when the file has
/// never been indexed. Behind `crate::domains::code::index_code::walk`'s incremental skip
/// gate — the caller compares this against the file's current blob OID.
pub fn blob_oid_for(conn: &Connection, repo: &str, path: &str) -> Result<Option<String>> {
    orm::query_optional(
        conn,
        repo_rows(&[&c::blob_oid], repo)
            .filter(c::path.eq(path))
            .to_sql(),
        |r| r.get(0),
    )
}

/// Decode one `(path, blob_oid)` row in [`list_for_repo`]'s column order.
fn path_and_oid(r: &rusqlite::Row<'_>) -> rusqlite::Result<(String, String)> {
    Ok((r.get(0)?, r.get(1)?))
}

/// Every `(path, blob_oid)` cursor row for `repo`, ascending by path — the
/// manifest `GET /sync/code/manifest` answers and the local side the code
/// push diffs it against.
pub fn list_for_repo(conn: &Connection, repo: &str) -> Result<Vec<(String, String)>> {
    orm::query_all(
        conn,
        repo_rows(&[&c::path, &c::blob_oid], repo)
            .order_by(c::path.asc())
            .to_sql(),
        path_and_oid,
    )
}

#[cfg(test)]
#[path = "tests/indexed_files.rs"]
mod tests;
