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

/// Run the coordinator in the foreground (`comemory sync daemon run`).
///
/// # Errors
/// As [`coordinator::run`].
pub async fn run_foreground(paths: &Paths) -> Result<()> {
    coordinator::run(paths).await
}

#[cfg(test)]
#[path = "tests/daemon.rs"]
mod tests;
