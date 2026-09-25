//! The required resident sync coordinator (#257): one per canonical data
//! directory, always running, verified and repaired on every ordinary CLI
//! start. Design: `docs/designs/2026-09-25-required-sync-daemon.md`.
//!
//! Folder index: [`daemon/README.md`](daemon/README.md).

/// The workspace channel thread.
pub mod channel;
/// Blocking control-socket client.
pub mod client;
/// Control protocol frames.
pub mod control;
/// The foreground coordinator: lock, bind, tasks, signals.
pub mod coordinator;
/// `ensure`/`restart`/`repair`/preflight over one data directory.
pub mod ensure;
/// `daemon.token` and the two-way proof.
pub mod handshake;
/// Canonical data directory, its id, and the running binary.
pub mod identity;
/// The owner-local readiness answer.
pub mod readiness;
/// `daemon.json`, the live coordinator's discovery record.
pub mod runtime_record;
/// The control server.
pub mod server;
/// Where the control socket lives, and ownership checks.
pub mod socket_path;
/// `Kind::Process` supervision: this build starts its own detached child.
pub mod spawn;
/// Shared coordinator state and events.
pub mod state;
/// `comemory sync daemon status`'s live probe report.
pub mod status_view;
/// Which OS supervisor keeps a data directory's coordinator running.
pub mod supervisor;
/// Socket binding, the reconciliation ticker and the guard.
pub mod watchdog;
/// The coalescing pass worker.
pub mod worker;

use crate::config::Paths;
use crate::prelude::*;

pub use crate::domains::sync::daemon_unit::{
    DAEMON_LABEL, DaemonStatus, SYSTEMD_UNIT, install, launch_agent_plist,
    render_launch_agent_plist, render_systemd_unit, start, status, stop, systemd_user_unit,
    uninstall,
};

/// Run the coordinator in the foreground (`comemory sync daemon run`).
///
/// # Errors
/// As [`coordinator::run`].
pub async fn run_foreground(paths: &Paths) -> Result<()> {
    coordinator::run(paths).await
}

/// Best-effort install+start used by `auth login` (never fails the login).
pub fn install_and_start_best_effort(paths: &Paths) {
    // Integration tests set this so login does not touch the host launchd /
    // systemd session while HOME is a tempfile.
    if crate::config::sync::daemon_disabled() {
        tracing::debug!("sync daemon skipped (COMEMORY_SYNC_DAEMON=0)");
        return;
    }
    match install(paths) {
        Ok(path) => tracing::info!(path = %path.display(), "sync daemon installed"),
        Err(e) => {
            tracing::warn!(error = %e, "sync daemon install failed");
            return;
        }
    }
    if let Err(e) = start() {
        tracing::warn!(error = %e, "sync daemon start failed");
    }
}

/// Best-effort stop used by `auth logout` (unit left installed).
pub fn stop_best_effort() {
    // The same guard as the install: a test's logout must not stop the
    // host's own daemon.
    if crate::config::sync::daemon_disabled() {
        tracing::debug!("sync daemon stop skipped (COMEMORY_SYNC_DAEMON=0)");
        return;
    }
    stop();
}

#[cfg(test)]
#[path = "tests/daemon.rs"]
mod tests;
