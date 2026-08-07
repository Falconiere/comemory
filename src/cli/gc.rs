//! `comemory gc` — purge entries in `memories/.trash/` older than 30 days
//! and evict learning telemetry (`retrieval_log`, `feedback_events`) past
//! the configured retention window (`prune.learning_retention_days`). The
//! sweep itself lives in `api::gc` (Binding Rule 1), including the
//! must-not-create-the-db-on-a-fresh-dir invariant.

use std::io::Write as _;
use std::path::PathBuf;

use crate::api::{self, Ctx};
use crate::cli::load_config;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::output::json;
use crate::prelude::*;

/// Example invocations shown at the bottom of `comemory gc --help`.
pub const EXAMPLES: &str = "\
Examples:
  # Hard-delete .trash entries and learning telemetry past retention
  comemory gc

  # Tighten the telemetry window (retrieval_log + feedback_events) to a week
  COMEMORY_LEARNING_RETENTION_DAYS=7 comemory gc

  # JSON output for CI/automation
  comemory gc --json";

/// Remove every file in the trash directory whose mtime is older than the
/// retention window, then evict learning telemetry older than
/// `prune.learning_retention_days` from `comemory.db` — but only when the db
/// file already exists (`api::gc::run` preserves this invariant). Missing
/// trash directory is a no-op. Reports the trash, `retrieval_log`, and
/// `feedback_events` removal counts.
pub async fn run(json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let cfg = load_config(&paths)?;
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let resp = api::gc::run(&mut ctx, api::gc::Request {})?;

    if json_flag {
        json::write(&resp)?;
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(
            out,
            "gc removed {} trashed memories, {} log rows, {} feedback events",
            resp.removed, resp.log_rows, resp.event_rows
        )?;
    }
    Ok(())
}
