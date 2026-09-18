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
use toolu_orm::core::query_column::CommonOps;

use rusqlite::Connection;

use crate::prelude::*;

/// Drop every `indexed_files` cursor row for `repo`, forcing the next
/// `index-code` walk to re-extract every file. Shared by
/// [`crate::store::code_row::ensure_repo_format`] (the code-format-version
/// gate) and `crate::domains::code::index_code::run_with_progress`'s `--mode full`.
pub fn delete_for_repo(conn: &Connection, repo: &str) -> Result<()> {
    orm::execute(
        conn,
        IndexedFiles::delete().filter(c::repo.eq(repo)).to_sql(),
    )?;
    Ok(())
}

/// The recorded blob OID for `(repo, path)`, or `None` when the file has
/// never been indexed. Behind `crate::domains::code::index_code::walk`'s incremental skip
/// gate — the caller compares this against the file's current blob OID.
pub fn blob_oid_for(conn: &Connection, repo: &str, path: &str) -> Result<Option<String>> {
    orm::query_optional(
        conn,
        IndexedFiles::select()
            .columns_typed(&[&c::blob_oid])
            .filter(c::repo.eq(repo))
            .filter(c::path.eq(path))
            .to_sql(),
        |r| r.get(0),
    )
}

/// Every `(path, blob_oid)` cursor row for `repo`, ascending by path — the
/// manifest `GET /sync/code/manifest` answers and the local side the code
/// push diffs it against.
pub fn list_for_repo(conn: &Connection, repo: &str) -> Result<Vec<(String, String)>> {
    orm::query_all(
        conn,
        IndexedFiles::select()
            .columns_typed(&[&c::path, &c::blob_oid])
            .filter(c::repo.eq(repo))
            .order_by(c::path.asc())
            .to_sql(),
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
}

/// Drop the cursor row for one `(repo, path)` — the removal half of a code
/// import; a path with no row is a no-op.
pub fn delete_one(conn: &Connection, repo: &str, path: &str) -> Result<()> {
    orm::execute(
        conn,
        IndexedFiles::delete()
            .filter(c::repo.eq(repo))
            .filter(c::path.eq(path))
            .to_sql(),
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/indexed_files.rs"]
mod tests;
