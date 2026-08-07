//! `api::gc::{Request, run}` — the shared middle of `comemory gc` / `POST
//! /api/v1/gc`: purge entries in `memories/.trash/` older than 30 days and
//! evict learning telemetry (`retrieval_log`, `feedback_events`) past the
//! configured retention window. Moved out of `cli::gc::run` (Binding Rule 1).
//!
//! **Must-not-create-the-db invariant:** `run` calls [`Ctx::conn`] only when
//! `comemory.db` already exists on disk — `gc` on a fresh data dir must never
//! create (and migrate) a db as a side effect.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::api::Ctx;
use crate::prelude::*;
use crate::store::memory_row;

/// `comemory gc` / `POST /api/v1/gc` request. No CLI args today.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {}

/// Removal counts from one `gc` run.
#[derive(Serialize, Debug)]
pub struct Response {
    /// Trashed memories hard-deleted (mtime past [`RETENTION_DAYS`]).
    pub removed: u64,
    /// `retrieval_log` rows evicted past the configured retention window.
    pub log_rows: u64,
    /// `feedback_events` rows evicted past the configured retention window.
    pub event_rows: u64,
}

/// Trash retention window, in days. Intentionally fixed in v1 (see module
/// doc on `cli::gc`).
const RETENTION_DAYS: i64 = 30;

/// Remove every file in the trash directory whose mtime is older than
/// [`RETENTION_DAYS`], then — only when `comemory.db` already exists — evict
/// learning telemetry older than `prune.learning_retention_days`. Missing
/// trash directory is a no-op.
pub fn run(ctx: &mut Ctx<'_>, _req: Request) -> Result<Response> {
    let mut removed = 0u64;

    if let Ok(rd) = std::fs::read_dir(ctx.paths.trash_dir()) {
        for entry in rd.flatten() {
            let too_old = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .map(|d| d > std::time::Duration::from_secs((RETENTION_DAYS as u64) * 86_400))
                .unwrap_or(false);
            if too_old && std::fs::remove_file(entry.path()).is_ok() {
                removed += 1;
            }
        }
    }

    let (log_rows, event_rows) = if ctx.paths.db_path().exists() {
        let retention_days = ctx.cfg.prune.learning_retention_days;
        let conn = ctx.conn()?;
        sweep_learning(conn, retention_days, OffsetDateTime::now_utc())?
    } else {
        (0, 0)
    };

    Ok(Response {
        removed,
        log_rows,
        event_rows,
    })
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
