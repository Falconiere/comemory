//! `api::gc::{Request, run}` — the shared middle of `comemory gc` / `POST
//! /api/v1/gc`: purge entries in `memories/.trash/` older than
//! `prune.trash_retention_days` (30 by default) and evict learning
//! telemetry (`retrieval_log`, `feedback_events`) past
//! `prune.learning_retention_days`. Moved out of `cli::gc::run` (Binding
//! Rule 1). Both windows are the `GET|PUT /api/v1/gc/policy` knobs.
//!
//! **Must-not-create-the-db invariant:** `run` calls [`Ctx::conn`] only when
//! `comemory.db` already exists on disk — `gc` on a fresh data dir must never
//! create (and migrate) a db as a side effect.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::api::Ctx;
use crate::prelude::*;
use crate::store::{gc_runs, memory_row, random_id};

/// `comemory gc` / `POST /api/v1/gc` request. No CLI args today.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {}

/// Removal counts from one `gc` run.
#[derive(Serialize, Debug)]
pub struct Response {
    /// Trashed memories hard-deleted (mtime past
    /// `prune.trash_retention_days`).
    pub removed: u64,
    /// `retrieval_log` rows evicted past the configured retention window.
    pub log_rows: u64,
    /// `feedback_events` rows evicted past the configured retention window.
    pub event_rows: u64,
    /// Summed size, in bytes, of the trashed files this run actually
    /// removed (stat'd before the unlink, never estimated after).
    pub bytes_freed: u64,
}

/// Random bytes behind a `gc_runs` row id — 8 bytes, rendered as 16
/// lowercase-hex chars (the same width as a job id).
const RUN_ID_BYTES: usize = 8;

/// Remove every file in the trash directory whose mtime is older than
/// `prune.trash_retention_days`, then — only when `comemory.db` already
/// exists — evict learning telemetry older than
/// `prune.learning_retention_days` AND record this run in `gc_runs`.
/// Missing trash directory is a no-op. The must-not-create-the-db invariant
/// means a fresh data dir writes no `gc_runs` row either — there is nowhere
/// to write it.
pub fn run(ctx: &mut Ctx<'_>, _req: Request) -> Result<Response> {
    let trash_days = i64::from(ctx.cfg.prune.trash_retention_days);
    let (removed, bytes_freed) = sweep_trash(&ctx.paths.trash_dir(), trash_days);

    let (log_rows, event_rows) = if ctx.paths.db_path().exists() {
        let retention_days = ctx.cfg.prune.learning_retention_days;
        let conn = ctx.conn()?;
        let now = OffsetDateTime::now_utc();
        let counts = sweep_learning(conn, retention_days, now)?;
        record_run(conn, removed, counts.0, counts.1, bytes_freed, now)?;
        counts
    } else {
        (0, 0)
    };

    Ok(Response {
        removed,
        log_rows,
        event_rows,
        bytes_freed,
    })
}

/// Remove every file directly under `trash_dir` whose mtime is older than
/// `retention_days`, returning `(files_removed, bytes_freed)`. Each file's
/// size is stat'd BEFORE the unlink, so a failed `remove_file` never counts
/// toward `bytes_freed`. Missing trash directory yields `(0, 0)`.
fn sweep_trash(trash_dir: &std::path::Path, retention_days: i64) -> (u64, u64) {
    let mut removed = 0u64;
    let mut bytes_freed = 0u64;
    let Ok(rd) = std::fs::read_dir(trash_dir) else {
        return (0, 0);
    };
    for entry in rd.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let too_old = meta
            .modified()
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|d| d > std::time::Duration::from_secs((retention_days as u64) * 86_400));
        if too_old && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
            bytes_freed += meta.len();
        }
    }
    (removed, bytes_freed)
}

/// Insert one `gc_runs` row for this completed sweep. Only reached when
/// `comemory.db` already exists (the caller's `conn` came from [`Ctx::conn`]
/// after that check).
fn record_run(
    conn: &Connection,
    removed: u64,
    log_rows: u64,
    event_rows: u64,
    bytes_freed: u64,
    now: OffsetDateTime,
) -> Result<()> {
    let id = random_id::random_hex(RUN_ID_BYTES)?;
    let at = memory_row::iso_format(now)?;
    gc_runs::insert(conn, &id, &at, removed, log_rows, event_rows, bytes_freed)
}

/// Evict learning telemetry older than the retention window. Counters in
/// `feedback` are permanent; only raw event rows age out.
///
/// Both `retrieval_log.at` and `feedback_events.at` are written via
/// [`memory_row::iso_format`] (`Iso8601::DEFAULT`), which renders a
/// fixed-width `YYYY-MM-DDTHH:MM:SS.nnnnnnnnnZ` string — always nine
/// fractional digits, verified empirically (whole-second values render as
/// `.000000000Z`, see the shape assertion in `tests/cli/gc.rs`). On
/// identical-width ISO-8601 UTC strings, lexicographic `<` is exactly
/// chronological, so a plain string comparison against the rendered cutoff
/// is correct without any `substr` truncation.
fn sweep_learning(
    conn: &Connection,
    retention_days: u32,
    now: OffsetDateTime,
) -> Result<(u64, u64)> {
    let cutoff = memory_row::iso_format(now - time::Duration::days(i64::from(retention_days)))?;
    let logs = conn.execute("DELETE FROM retrieval_log WHERE at < ?1", [&cutoff])? as u64;
    let events = conn.execute("DELETE FROM feedback_events WHERE at < ?1", [&cutoff])? as u64;
    Ok((logs, events))
}

#[cfg(test)]
#[path = "tests/gc.rs"]
mod tests;
