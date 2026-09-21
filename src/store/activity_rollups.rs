//! Per-command rollups over `activity_log`: runs, failures, and the p50/p95
//! of `duration_ms`.
//!
//! Counts come from `COUNT(*)` under the caller's filter, so `runs` and
//! `errors` are exact. The percentiles are computed in Rust over at most
//! [`SAMPLE`] of the newest durations per command: SQLite has no percentile
//! function, and expressing one through window functions would mean hand SQL
//! inside `store/` for a number a console renders as a bar.

use rusqlite::Connection;
use serde::Serialize;
use toolu_orm::core::query_column::CommonOps;

use super::activity::{ActivityFilter, apply_filter};
use super::{
    orm,
    schema_history::{ActivityLog, activity_log as col},
};
use crate::prelude::*;

/// Newest durations per command the percentiles are computed over.
pub const SAMPLE: usize = 1_000;

/// One command's slice of the filtered window.
#[derive(Debug, Clone, Serialize)]
pub struct ActivityRollup {
    /// The command these counts describe.
    pub command: String,
    /// Runs recorded under the caller's filter.
    pub runs: u64,
    /// How many of them failed (`ok = 0`).
    pub errors: u64,
    /// Median duration over the sampled window, `0` when nothing was sampled.
    pub p50_ms: i64,
    /// 95th-percentile duration over the sampled window.
    pub p95_ms: i64,
}

/// Roll the filtered window up per command, ordered by descending run count
/// so the busiest command leads. One `COUNT(*)` pair and one duration read
/// per distinct command — a dozen commands exist, so the loop is bounded by
/// the vocabulary, not by the table.
pub fn rollups(conn: &Connection, filter: &ActivityFilter<'_>) -> Result<Vec<ActivityRollup>> {
    let mut out = Vec::new();
    for command in distinct_commands(conn, filter)? {
        let mut scoped = *filter;
        scoped.command = Some(&command);
        let runs = count(conn, &scoped, false)?;
        let errors = count(conn, &scoped, true)?;
        let durations = sample_durations(conn, &scoped)?;
        out.push(ActivityRollup {
            command: command.clone(),
            runs,
            errors,
            p50_ms: percentile(&durations, 50),
            p95_ms: percentile(&durations, 95),
        });
    }
    out.sort_by(|a, b| b.runs.cmp(&a.runs).then_with(|| a.command.cmp(&b.command)));
    Ok(out)
}

/// The distinct commands present in the filtered window.
fn distinct_commands(conn: &Connection, filter: &ActivityFilter<'_>) -> Result<Vec<String>> {
    orm::query_all(
        conn,
        apply_filter(
            ActivityLog::select()
                .columns_typed(&[&col::command])
                .distinct(),
            filter,
        )
        .to_sql(),
        |r| r.get::<_, String>(0),
    )
}

/// `COUNT(*)` under `filter`, optionally restricted to failed runs.
fn count(conn: &Connection, filter: &ActivityFilter<'_>, failed_only: bool) -> Result<u64> {
    let mut query = apply_filter(ActivityLog::select(), filter);
    if failed_only {
        query = query.filter(col::ok.eq(0_i64));
    }
    let total: i64 = orm::query_one(conn, query.to_count_sql(), |r| r.get(0))?;
    Ok(u64::try_from(total).unwrap_or(0))
}

/// The newest [`SAMPLE`] durations under `filter`, ascending — ready for
/// [`percentile`] to index without re-sorting the whole window.
fn sample_durations(conn: &Connection, filter: &ActivityFilter<'_>) -> Result<Vec<i64>> {
    let mut durations = orm::query_all(
        conn,
        apply_filter(
            ActivityLog::select().columns_typed(&[&col::duration_ms]),
            filter,
        )
        .order_by(col::id.desc())
        .limit(i64::try_from(SAMPLE).unwrap_or(i64::MAX))
        .to_sql(),
        |r| r.get::<_, i64>(0),
    )?;
    durations.sort_unstable();
    Ok(durations)
}

/// Nearest-rank percentile over an ascending slice; `0` when empty.
fn percentile(sorted: &[i64], p: usize) -> i64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = p.saturating_mul(sorted.len()).div_ceil(100);
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted.get(index).copied().unwrap_or(0)
}

#[cfg(test)]
#[path = "tests/activity_rollups.rs"]
mod tests;
