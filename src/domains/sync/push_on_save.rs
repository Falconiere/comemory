//! Push the outbox inline, right after a local write
//! (`2026-09-14-sync-everything-realtime-design.md`).
//!
//! This is what lets continuous sync stop depending on a resident process. A
//! push is event-driven — something was written, and the command that wrote it
//! is still running — so the writing command pays for it, and the daemon is
//! left to the one direction that genuinely needs polling.
//!
//! Three properties the caller depends on, in order of importance:
//!
//! 1. **It never fails a write.** The markdown is already on disk and the row
//!    already committed by the time this runs. A network that is down, a
//!    credential that is missing, a platform that refuses — none of it may turn
//!    a successful save into a failed command, so nothing here returns `Err`.
//! 2. **It is bounded.** `[sync] push_on_save_timeout` (2s by default) caps the
//!    HTTP call, because a captive portal answers the connection and then says
//!    nothing for as long as you let it.
//! 3. **It loses nothing.** The outbox is durable and the drain sends it in
//!    order, so a push that never happened costs latency only: the next
//!    `comemory sync`, `comemory watch`, or daemon cycle sends the same entry.
//! 4. **It never waits for a pass.** It takes the sync pass lock with `try`
//!    and skips when a pass holds it; the running pass sends the new
//!    operation on its next push batch.
//!
//! It lives in `sync::` but is called from `cli::` — through
//! [`crate::cli::off_runtime::off_runtime`] — never from a command core. `domains::memories::save`
//! also runs inside `comemory serve`, where `reqwest::blocking` would panic on
//! drop inside the tokio runtime, and where a server pushing its tenants'
//! memories outward would be wrong anyway.

use std::time::Duration;

use crate::config::{Config, Paths};
use crate::domains::sync::AuthFile;
use crate::domains::sync::auto::PASS_LOCK;
use crate::domains::sync::daemon::client;
use crate::domains::sync::daemon::control::Wake;
use crate::domains::sync::daemon::readiness::Trigger;
use crate::domains::sync::drain::{
    self,
    report::{End, Report},
    session::{Legs, Mode},
};
use crate::prelude::*;
use crate::store::connection;
use crate::utilities::file_lock::FileLock;

/// How long the best-effort wake may take; well under a save's own budget.
const WAKE_BOUND: Duration = Duration::from_millis(200);

/// Push the outbox after a local write. Never fails, never panics, never
/// blocks longer than `[sync] push_on_save_timeout` per request.
///
/// When `[sync] push_on_save` is off, or the inline attempt did not fully
/// drain the outbox (disabled, skipped, failed, or `more` left), a
/// best-effort wake asks the resident coordinator to finish the job at once
/// rather than waiting for its next reconciliation tick — the same
/// coordinator every ordinary command's preflight already keeps healthy.
pub fn after_write_best_effort(paths: &Paths, cfg: &Config) {
    let outcome = if cfg.sync.push_on_save {
        try_push(paths, cfg)
    } else {
        Ok(None)
    };
    match &outcome {
        Ok(Some(report)) => tracing::debug!(
            pushed = report.pushed,
            held = report.held,
            end = ?report.end,
            "inline push after write"
        ),
        Ok(None) => tracing::debug!("inline push skipped: not logged in, or a pass is running"),
        // Deliberately not a warning: being offline is an ordinary state for a
        // laptop, and `comemory sync --action status` reports what is pending.
        Err(e) => tracing::debug!(error = %e, "inline push after write failed"),
    }
    if !fully_drained(&outcome) {
        let _ = client::wake(
            paths,
            Wake {
                checkout: None,
                reason: Trigger::Save,
            },
            WAKE_BOUND,
        );
    }
}

/// Whether the inline attempt already emptied the outbox, so no wake is
/// worth sending.
fn fully_drained(outcome: &Result<Option<Report>>) -> bool {
    matches!(outcome, Ok(Some(report)) if report.end == End::CaughtUp)
}

/// The fallible half. `Ok(None)` means there was no credential, or a pass
/// holds the sync lock — that pass sends this write on its next push batch.
fn try_push(paths: &Paths, cfg: &Config) -> Result<Option<Report>> {
    let Some(auth) = AuthFile::load_usable(paths)? else {
        return Ok(None);
    };
    let Some(_pass) = FileLock::try_acquire(&paths.data_dir().join(PASS_LOCK), "sync")? else {
        return Ok(None);
    };
    let timeout = cfg.sync.push_on_save_timeout_duration()?;
    let mut conn = connection::open(paths.db_path())?;
    let drained = drain::drain(
        paths,
        cfg,
        &mut conn,
        &auth,
        (Mode::Inline(timeout), Legs::Push),
    )?;
    Ok(Some(drained.exchange))
}

#[cfg(test)]
#[path = "tests/push_on_save.rs"]
mod tests;
