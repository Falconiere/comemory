//! `activity_log` insert + reads: the single write behind every instrumented
//! command run (`utilities::activity::record`), the filtered newest-first page
//! behind `GET /api/v1/activity`, the ascending cursor read the SSE stream
//! polls, and the retention delete `maintenance::gc` sweeps with.
//!
//! Every read takes the same [`ActivityFilter`], so the snapshot route and the
//! stream cannot disagree about what a filter means.

use rusqlite::Connection;
use serde::Serialize;
use toolu_orm::core::expr::Scalar;
use toolu_orm::core::query_column::{CommonOps, NumericOps};
use toolu_orm::query::select::SelectBuilder;

use super::{
    orm,
    schema_history::{ActivityLog, activity_log as col},
};
use crate::prelude::*;

/// Insert parameters for one `activity_log` row, bundled into a struct rather
/// than nine positional arguments (`clippy::too_many_arguments`).
pub struct NewActivityRow<'a> {
    /// Pre-rendered ISO-8601 UTC timestamp (`store::memory_row::iso_format`).
    pub at: &'a str,
    /// The command that ran (`utilities::activity::command` const).
    pub command: &'a str,
    /// `cli` | `http` | `mcp` (matches the table's `CHECK`).
    pub source: &'a str,
    /// Caller label the caller declared, `None` when it declared none.
    pub actor: Option<&'a str>,
    /// Repo label the run was scoped to, `None` when unscoped.
    pub repo: Option<&'a str>,
    /// Wall-clock duration of the core call.
    pub duration_ms: i64,
    /// Whether the core returned `Ok`.
    pub ok: bool,
    /// `utilities::error_code::classify` slug when `ok` is false.
    pub error_code: Option<&'a str>,
    /// Pre-serialized JSON summary, `None` when summaries are off.
    pub summary: Option<&'a str>,
    /// Device that ran the command; `None` for a run recorded here.
    pub device: Option<&'a str>,
    /// Replica event id, for an imported run (#254).
    pub event_id: Option<&'a str>,
}

/// One `activity_log` row as the read surfaces return it. `summary` stays the
/// stored JSON text: parsing (and warning about a malformed row) belongs to
/// `domains::maintenance::activity`, not to the store.
#[derive(Debug, Clone, Serialize)]
pub struct ActivityRow {
    /// Monotonic row id; the stream's cursor.
    pub id: i64,
    /// ISO-8601 UTC timestamp.
    pub at: String,
    /// The command that ran.
    pub command: String,
    /// `cli` | `http` | `mcp`.
    pub source: String,
    /// Caller label, when one was declared.
    pub actor: Option<String>,
    /// Repo label, when the run was scoped.
    pub repo: Option<String>,
    /// Wall-clock duration of the core call.
    pub duration_ms: i64,
    /// Whether the core returned `Ok`.
    pub ok: bool,
    /// Error slug when `ok` is false.
    pub error_code: Option<String>,
    /// Stored JSON summary text, when one was written.
    pub summary: Option<String>,
    /// Device that ran the command; `None` for a run recorded here (#254).
    pub device: Option<String>,
}

/// What a read narrows to. Every field is `None` by default — an all-`None`
/// filter is "everything".
#[derive(Debug, Clone, Copy, Default)]
pub struct ActivityFilter<'a> {
    /// Only runs scoped to this repo label.
    pub repo: Option<&'a str>,
    /// Only this command.
    pub command: Option<&'a str>,
    /// Only this delivery surface.
    pub source: Option<&'a str>,
    /// Only this caller label.
    pub actor: Option<&'a str>,
    /// Only runs at or after this ISO-8601 UTC timestamp.
    pub since: Option<&'a str>,
    /// Only rows above this id (the stream cursor).
    pub after_id: Option<i64>,
}

/// The projection both readers share, so a column added to one cannot go
/// missing from the other.
pub(super) fn select_rows() -> SelectBuilder {
    ActivityLog::select().columns_typed(&[
        &col::id,
        &col::at,
        &col::command,
        &col::source,
        &col::actor,
        &col::repo,
        &col::duration_ms,
        &col::ok,
        &col::error_code,
        &col::summary,
        &col::device,
    ])
}

/// Narrow a query by every set field of `filter`.
pub(super) fn apply_filter(mut query: SelectBuilder, filter: &ActivityFilter<'_>) -> SelectBuilder {
    if let Some(repo) = filter.repo {
        query = query.filter(col::repo.eq(repo));
    }
    if let Some(command) = filter.command {
        query = query.filter(col::command.eq(command));
    }
    if let Some(source) = filter.source {
        query = query.filter(col::source.eq(source));
    }
    if let Some(actor) = filter.actor {
        query = query.filter(col::actor.eq(actor));
    }
    if let Some(since) = filter.since {
        // `at` is `Text`, so the ordering comparison goes through `Scalar`
        // rather than `NumericOps` (implemented only for the numeric and
        // instant column types). Both sides are `iso_format`-shaped, so a
        // plain string `>=` compares chronologically.
        query = query.filter(Scalar::col(&col::at).gte(Scalar::bind(since)));
    }
    if let Some(after_id) = filter.after_id {
        query = query.filter(col::id.gt(after_id));
    }
    query
}

/// Insert one row and return its id — the id the SSE stream hands clients as
/// its cursor, so it is read back from the same connection that wrote it
/// rather than re-queried.
pub fn insert(conn: &Connection, row: &NewActivityRow<'_>) -> Result<i64> {
    orm::execute(
        conn,
        ActivityLog::insert()
            .set(&col::at, row.at)
            .set(&col::command, row.command)
            .set(&col::source, row.source)
            .set(&col::actor, row.actor)
            .set(&col::repo, row.repo)
            .set(&col::duration_ms, row.duration_ms)
            .set(&col::ok, i64::from(row.ok))
            .set(&col::error_code, row.error_code)
            .set(&col::summary, row.summary)
            .set(&col::device, row.device)
            .set(&col::event_id, row.event_id)
            .to_sql(),
    )?;
    Ok(conn.last_insert_rowid())
}

/// A `(limit, offset)` window of runs, newest-first (`at DESC, id DESC`,
/// matching `idx_activity_log_at`), plus the total row count under the same
/// filter. `limit == 0` is the shared "all" sentinel.
pub fn list(
    conn: &Connection,
    filter: &ActivityFilter<'_>,
    limit: usize,
    offset: usize,
) -> Result<(Vec<ActivityRow>, usize)> {
    // `SelectBuilder` is not `Clone`, so the count and the page each build
    // their own query from the same filter.
    let total: i64 = orm::query_one(
        conn,
        apply_filter(ActivityLog::select(), filter).to_count_sql(),
        |r| r.get(0),
    )?;
    let query = apply_filter(select_rows(), filter);
    let limit = if limit == 0 {
        -1
    } else {
        i64::try_from(limit).unwrap_or(i64::MAX)
    };
    let rows = orm::query_all(
        conn,
        query
            .order_by(col::at.desc())
            .order_by(col::id.desc())
            .limit(limit)
            .offset(i64::try_from(offset).unwrap_or(i64::MAX))
            .to_sql(),
        row_from_query,
    )?;
    Ok((rows, usize::try_from(total).unwrap_or(0)))
}

/// Rows above `filter.after_id` in ascending id order — what the SSE handler
/// polls. Ascending on purpose: the client's cursor advances one row at a
/// time and must never skip a row that arrived between two polls.
pub fn since_cursor(
    conn: &Connection,
    filter: &ActivityFilter<'_>,
    limit: usize,
) -> Result<Vec<ActivityRow>> {
    let query = apply_filter(select_rows(), filter);
    orm::query_all(
        conn,
        query
            .order_by(col::id.asc())
            .limit(i64::try_from(limit).unwrap_or(i64::MAX))
            .to_sql(),
        row_from_query,
    )
}

/// The newest row id, or `0` on an empty table — the stream's connect-time
/// cursor, so a client is not replayed the whole history on connect.
pub fn newest_id(conn: &Connection) -> Result<i64> {
    let row = orm::query_optional(
        conn,
        ActivityLog::select()
            .columns_typed(&[&col::id])
            .order_by(col::id.desc())
            .limit(1)
            .to_sql(),
        |r| r.get::<_, i64>(0),
    )?;
    Ok(row.unwrap_or(0))
}

/// Delete rows older than `cutoff` (exclusive), returning how many went —
/// the retention sweep `maintenance::gc` runs.
pub fn delete_before(conn: &Connection, cutoff: &str) -> Result<u64> {
    let removed = orm::execute(
        conn,
        ActivityLog::delete()
            .filter(Scalar::col(&col::at).lt(Scalar::bind(cutoff)))
            .to_sql(),
    )?;
    Ok(u64::try_from(removed).unwrap_or(0))
}

/// Map one projected row into an [`ActivityRow`].
pub(super) fn row_from_query(r: &rusqlite::Row<'_>) -> rusqlite::Result<ActivityRow> {
    Ok(ActivityRow {
        id: r.get(0)?,
        at: r.get(1)?,
        command: r.get(2)?,
        source: r.get(3)?,
        actor: r.get(4)?,
        repo: r.get(5)?,
        duration_ms: r.get(6)?,
        ok: r.get::<_, i64>(7)? != 0,
        error_code: r.get(8)?,
        summary: r.get(9)?,
        device: r.get(10)?,
    })
}

#[cfg(test)]
#[path = "tests/activity.rs"]
mod tests;
