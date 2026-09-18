//! `index_runs` row insert + readers — one row per `comemory index-code`
//! RUN, outcomes included (`ok` | `error` | `cancelled`), backing the v15
//! console-API history table (`migrations/0015_v15_console_api.sql`).
//! `GET /api/v1/index/runs` pages it; `GET /api/v1/overview` reads the
//! newest row for its "last run" tile.

use super::{
    orm,
    schema_history::{IndexRuns, index_runs as c},
};
use rusqlite::Connection;
use serde::Serialize;
use toolu_orm::core::query_column::CommonOps;

use crate::prelude::*;

/// Shared projection and stable ordering for run readers.
fn select_runs() -> toolu_orm::query::select::SelectBuilder {
    IndexRuns::select()
        .columns_typed(&[
            &c::id,
            &c::repo,
            &c::root_path,
            &c::mode,
            &c::started_at,
            &c::finished_at,
            &c::duration_ms,
            &c::files_indexed,
            &c::symbols,
            &c::outcome,
            &c::error,
        ])
        .order_by(c::started_at.desc())
        .order_by(c::id.asc())
}

/// Insert parameters for one completed run, bundled into a struct rather
/// than eleven positional arguments (`clippy::too_many_arguments`).
pub struct NewIndexRun<'a> {
    /// 16-hex row id (`store::random_id::random_hex(8)`).
    pub id: &'a str,
    /// The repo label the run indexed.
    pub repo: &'a str,
    /// The working-tree root walked, when it canonicalized.
    pub root_path: Option<&'a str>,
    /// `"full"` | `"incremental"` (matches the table's `CHECK`).
    pub mode: &'a str,
    /// Pre-rendered ISO-8601 UTC start timestamp (`memory_row::iso_format`).
    pub started_at: &'a str,
    /// Pre-rendered ISO-8601 UTC finish timestamp.
    pub finished_at: &'a str,
    /// Wall-clock duration of the run.
    pub duration_ms: u64,
    /// Files actually (re)indexed by this run.
    pub files_indexed: u64,
    /// `code_symbols` rows for the repo after the run.
    pub symbols: u64,
    /// `"ok"` | `"error"` | `"cancelled"` (matches the table's `CHECK`).
    pub outcome: &'a str,
    /// The failure message for an `error` outcome, else `None`.
    pub error: Option<&'a str>,
}

/// One `index_runs` row, as returned by [`list`] and [`newest`].
#[derive(Debug, Clone, Serialize)]
pub struct IndexRunRow {
    /// Row id.
    pub id: String,
    /// The repo label the run indexed.
    pub repo: String,
    /// The working-tree root walked, when recorded.
    pub root_path: Option<String>,
    /// `"full"` | `"incremental"`.
    pub mode: String,
    /// ISO-8601 UTC start timestamp.
    pub started_at: String,
    /// ISO-8601 UTC finish timestamp.
    pub finished_at: String,
    /// Wall-clock duration of the run.
    pub duration_ms: u64,
    /// Files actually (re)indexed by this run.
    pub files_indexed: u64,
    /// `code_symbols` rows for the repo after the run.
    pub symbols: u64,
    /// `"ok"` | `"error"` | `"cancelled"`.
    pub outcome: String,
    /// The failure message for an `error` outcome, else `None`.
    pub error: Option<String>,
}

/// Insert one `index_runs` row. A single `INSERT` with no read-modify-write
/// race — every field is caller-computed.
pub fn insert(conn: &Connection, row: &NewIndexRun<'_>) -> Result<()> {
    orm::execute(
        conn,
        IndexRuns::insert()
            .set(&c::id, row.id)
            .set(&c::repo, row.repo)
            .set(&c::root_path, row.root_path)
            .set(&c::mode, row.mode)
            .set(&c::started_at, row.started_at)
            .set(&c::finished_at, row.finished_at)
            .set(&c::duration_ms, clamp(row.duration_ms))
            .set(&c::files_indexed, clamp(row.files_indexed))
            .set(&c::symbols, clamp(row.symbols))
            .set(&c::outcome, row.outcome)
            .set(&c::error, row.error)
            .to_sql(),
    )?;
    Ok(())
}

/// A `(limit, offset)` window of runs, newest-first (`started_at DESC`,
/// matching `idx_index_runs_started`), narrowed to `repo` when one is
/// given, plus the total row count under the same filter. `limit == 0` is
/// the shared "all" sentinel.
pub fn list(
    conn: &Connection,
    repo: Option<&str>,
    limit: usize,
    offset: usize,
) -> Result<(Vec<IndexRunRow>, usize)> {
    let mut query = select_runs();
    if let Some(repo) = repo {
        query = query.filter(c::repo.eq(repo));
    }
    let total: i64 = orm::query_one(conn, query.to_count_sql(), |r| r.get(0))?;
    let limit = if limit == 0 {
        -1
    } else {
        i64::try_from(limit).unwrap_or(i64::MAX)
    };
    let rows = orm::query_all(
        conn,
        query
            .limit(limit)
            .offset(i64::try_from(offset).unwrap_or(i64::MAX))
            .to_sql(),
        row_from_query,
    )?;
    Ok((rows, usize::try_from(total).unwrap_or(0)))
}

/// The newest run on record, across every repo, or `None` on an empty
/// table.
pub fn newest(conn: &Connection) -> Result<Option<IndexRunRow>> {
    orm::query_optional(conn, select_runs().limit(1).to_sql(), row_from_query)
}

/// Map one projected row into an [`IndexRunRow`].
fn row_from_query(r: &rusqlite::Row<'_>) -> rusqlite::Result<IndexRunRow> {
    Ok(IndexRunRow {
        id: r.get(0)?,
        repo: r.get(1)?,
        root_path: r.get(2)?,
        mode: r.get(3)?,
        started_at: r.get(4)?,
        finished_at: r.get(5)?,
        duration_ms: unsigned(r.get(6)?),
        files_indexed: unsigned(r.get(7)?),
        symbols: unsigned(r.get(8)?),
        outcome: r.get(9)?,
        error: r.get(10)?,
    })
}

/// Saturate a `u64` count into SQLite's `i64` column type.
fn clamp(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// Read a stored count back as `u64`, treating a (never-written) negative
/// as zero.
fn unsigned(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

#[cfg(test)]
#[path = "tests/index_runs.rs"]
mod tests;
