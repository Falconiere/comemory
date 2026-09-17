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
//! 3. **It loses nothing.** `sync_log` is the outbox and `run_push` drains it
//!    from a cursor, so a push that never happened costs latency only: the next
//!    `comemory sync`, `comemory watch`, or daemon cycle sends the same entry.
//!
//! It lives in `sync::` but is called from `cli::` — through
//! [`crate::cli::off_runtime::off_runtime`] — never from `api::`. `domains::memories::save`
//! also runs inside `comemory serve`, where `reqwest::blocking` would panic on
//! drop inside the tokio runtime, and where a server pushing its tenants'
//! memories outward would be wrong anyway.

use crate::config::{Config, Paths};
use crate::domains::sync::AuthFile;
use crate::domains::sync::push;
use crate::prelude::*;
use crate::store::connection;

/// Entries one inline push may send. A single write adds one `sync_log` row;
/// the rest of the budget drains whatever earlier writes left behind, which is
/// what makes a machine that has been offline catch up on its next save.
const INLINE_LIMIT: usize = 100;

/// Push the outbox after a local write. Never fails, never panics, never
/// blocks longer than `[sync] push_on_save_timeout`.
pub fn after_write_best_effort(paths: &Paths, cfg: &Config) {
    if !cfg.sync.push_on_save {
        return;
    }
    match try_push(paths, cfg) {
        Ok(Some(stats)) => tracing::debug!(
            pushed = stats.pushed,
            skipped_config = stats.skipped_config,
            blocked_secrets = stats.blocked_secrets,
            "inline push after write"
        ),
        Ok(None) => tracing::debug!("inline push skipped: not logged in"),
        // Deliberately not a warning: being offline is an ordinary state for a
        // laptop, and `comemory sync --action status` reports what is pending.
        Err(e) => tracing::debug!(error = %e, "inline push after write failed"),
    }
}

/// The fallible half. `Ok(None)` means there was no credential to push with.
fn try_push(paths: &Paths, cfg: &Config) -> Result<Option<push::PushStats>> {
    let Some(auth) = AuthFile::load_usable(paths)? else {
        return Ok(None);
    };
    let timeout = cfg.sync.push_on_save_timeout_duration()?;
    let mut conn = connection::open(paths.db_path())?;
    let stats =
        push::run_push_with_timeout(paths, cfg, &mut conn, &auth, None, INLINE_LIMIT, timeout)?;
    Ok(Some(stats))
}

#[cfg(test)]
#[path = "tests/push_on_save.rs"]
mod tests;
