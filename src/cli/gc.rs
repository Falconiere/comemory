//! `comemory gc` — purge entries in `memories/.trash/` older than 30 days
//! and evict learning telemetry (`retrieval_log`, `feedback_events`) past
//! the configured retention window (`prune.learning_retention_days`).
//!
//! The trash sweep uses filesystem mtime (`std::fs::Metadata::modified`)
//! rather than a frontmatter-derived cutoff timestamp; this keeps gc working
//! even on trash entries whose frontmatter is unparsable, and avoids
//! re-reading every file. The 30-day trash retention is intentionally fixed
//! in v1, leaving the door open for swapping in a frontmatter-aware sweep
//! later.
//!
//! The telemetry sweep only opens `comemory.db` when the file already
//! exists — gc on a fresh data dir must not create (and migrate) a db as a
//! side effect. Aggregated `feedback` counters and mined `query_expansions`
//! are distilled knowledge and are never swept; only raw event rows age out.

use std::io::Write as _;
use std::path::PathBuf;

use rusqlite::Connection;
use time::OffsetDateTime;

use crate::cli::{load_config, resolve_data_dir};
use crate::config::paths::Paths;
use crate::output::json;
use crate::prelude::*;
use crate::store::memory_row;

/// Example invocations shown at the bottom of `comemory gc --help`.
pub const EXAMPLES: &str = "\
Examples:
  # Hard-delete .trash entries and learning telemetry past retention
  comemory gc

  # Tighten the telemetry window (retrieval_log + feedback_events) to a week
  COMEMORY_LEARNING_RETENTION_DAYS=7 comemory gc

  # JSON output for CI/automation
  comemory gc --json";

const RETENTION_DAYS: i64 = 30;

/// Remove every file in the trash directory whose mtime is older than the
/// retention window (`RETENTION_DAYS` days), then evict learning telemetry
/// older than `prune.learning_retention_days` from `comemory.db` — but only
/// when the db file already exists. Missing trash directory is a no-op.
/// Reports the trash, `retrieval_log`, and `feedback_events` removal counts.
pub async fn run(json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let cfg = load_config(&paths)?;
    let mut removed = 0u64;

    if let Ok(rd) = std::fs::read_dir(paths.trash_dir()) {
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

    let (log_rows, event_rows) = if paths.db_path().exists() {
        let conn = crate::store::connection::open(paths.db_path())?;
        sweep_learning(
            &conn,
            cfg.prune.learning_retention_days,
            OffsetDateTime::now_utc(),
        )?
    } else {
        (0, 0)
    };

    if json_flag {
        json::write(&serde_json::json!({
            "removed": removed,
            "log_rows": log_rows,
            "event_rows": event_rows,
        }))?;
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(
            out,
            "gc removed {removed} trashed memories, {log_rows} log rows, {event_rows} feedback events"
        )?;
    }
    Ok(())
}

/// Evict learning telemetry older than the retention window. Counters
/// in `feedback` are permanent; only raw event rows age out.
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
