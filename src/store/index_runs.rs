//! `index_runs` row insert + readers — one row per `comemory index-code`
//! RUN, outcomes included (`ok` | `error` | `cancelled`), backing the v15
//! console-API history table (`src/store/sql/0015_v15_console_api.sql`).
//! `GET /api/v1/index/runs` pages it; `GET /api/v1/overview` reads the
//! newest row for its "last run" tile.

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use crate::prelude::*;

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
    conn.execute(
        "INSERT INTO index_runs(id, repo, root_path, mode, started_at, finished_at, \
                                duration_ms, files_indexed, symbols, outcome, error) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            row.id,
            row.repo,
            row.root_path,
            row.mode,
            row.started_at,
            row.finished_at,
            clamp(row.duration_ms),
            clamp(row.files_indexed),
            clamp(row.symbols),
            row.outcome,
            row.error,
        ],
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
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM index_runs WHERE (?1 IS NULL OR repo = ?1)",
        [repo],
        |r| r.get(0),
    )?;
    let limit_param: i64 = if limit == 0 {
        -1
    } else {
        i64::try_from(limit).unwrap_or(i64::MAX)
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM index_runs WHERE (?1 IS NULL OR repo = ?1) \
          ORDER BY started_at DESC, id ASC LIMIT ?2 OFFSET ?3"
    ))?;
    let rows = stmt
        .query_map(
            rusqlite::params![repo, limit_param, i64::try_from(offset).unwrap_or(i64::MAX)],
            row_from_query,
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok((rows, usize::try_from(total).unwrap_or(0)))
}

/// The newest run on record, across every repo, or `None` on an empty
/// table.
pub fn newest(conn: &Connection) -> Result<Option<IndexRunRow>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM index_runs ORDER BY started_at DESC, id ASC LIMIT 1"),
        [],
        row_from_query,
    )
    .optional()
    .map_err(Error::from)
}

/// The projected column list every reader shares, in [`row_from_query`]'s
/// index order.
const COLUMNS: &str = "id, repo, root_path, mode, started_at, finished_at, duration_ms, \
                       files_indexed, symbols, outcome, error";

/// Map one [`COLUMNS`] row into an [`IndexRunRow`].
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
