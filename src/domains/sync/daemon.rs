//! Optional user-level OS daemon that periodically runs the
//! `sync --action auto` pass: re-index stale hooked repos, then pull and push
//! (logged out, only the re-index).
//!
//! Opt-in (`comemory auth login --daemon`); logout stops it, the unit stays.
//! Saves push inline, git and agent hooks fire the same pass, and `comemory
//! watch` covers live pulls, so what is left for a daemon is a headless host.
//!
//! Unit install/start/stop/status live in [`crate::domains::sync::daemon_unit`].

/// Blocking control-socket client.
pub mod client;
/// Control protocol frames.
pub mod control;
/// `daemon.token` and the two-way proof.
pub mod handshake;
/// Canonical data directory, its id, and the running binary.
pub mod identity;
/// The owner-local readiness answer.
pub mod readiness;
/// `daemon.json`, the live coordinator's discovery record.
pub mod runtime_record;
/// Where the control socket lives, and ownership checks.
pub mod socket_path;

use std::time::{Duration, Instant};

use crate::config::{Config, Paths};
use crate::domains::code::hooked_refresh::RefreshStats;
use crate::domains::sync::AuthFile;
use crate::domains::sync::daemon_unit;
use crate::domains::sync::{auto, verify};
use crate::prelude::*;
use crate::store::connection::open;

pub use daemon_unit::{
    DAEMON_LABEL, DaemonStatus, SYSTEMD_UNIT, install, launch_agent_plist,
    render_launch_agent_plist, render_systemd_unit, start, status, stop, systemd_user_unit,
    uninstall,
};

/// Load config the same way the CLI does (file overlay + env).
fn load_daemon_config(paths: &Paths) -> Result<Config> {
    Config::defaults()
        .with_file(paths.config_file().as_path())?
        .with_env()
}

/// Foreground loop: pull then push on `daemon_interval`, verify on `verify_every`.
///
/// Never returns successfully — the supervisor restarts on exit. Errors in a
/// cycle are logged and the loop continues.
pub fn run_foreground(paths: &Paths) -> Result<()> {
    let cfg = load_daemon_config(paths)?;
    let interval = cfg.sync.daemon_interval_duration()?;
    let verify_every = cfg.sync.verify_every_duration()?;
    // Force a verify on the first cycle after start so a fresh login gets an
    // integrity pass without waiting a week.
    let mut last_verify = Instant::now()
        .checked_sub(verify_every)
        .unwrap_or_else(Instant::now);

    tracing::info!(
        interval_secs = interval.as_secs(),
        verify_every_secs = verify_every.as_secs(),
        "sync daemon running"
    );

    loop {
        if let Err(e) = run_one_cycle(paths, &cfg, &mut last_verify, verify_every) {
            tracing::warn!(error = %e, "sync daemon cycle failed");
        }
        // A plain sleep: the wake file is gone with the 2026-09-14 sync
        // design, because a local save now pushes itself rather than nudging a
        // daemon to do it. This loop exists for the pull direction and for
        // headless hosts.
        std::thread::sleep(interval);
    }
}

/// One daemon cycle: the same pass `comemory sync --action auto` runs —
/// refresh every stale hooked repo, then pull, push, and push moved code —
/// under the sync pass lock, then an integrity verify when one is due.
fn run_one_cycle(
    paths: &Paths,
    cfg: &Config,
    last_verify: &mut Instant,
    verify_every: Duration,
) -> Result<()> {
    let _pass = auto::hold_pass_lock(paths)?;
    let mut conn = open(paths.db_path())?;
    auto::run_pass(paths, cfg, &mut conn, None, RefreshStats::default())?;
    let Some(auth) = AuthFile::load_usable(paths)? else {
        tracing::debug!("sync daemon: not logged in — network legs skipped");
        return Ok(());
    };
    if last_verify.elapsed() >= verify_every {
        match verify::verify(paths, cfg, &mut conn, &auth) {
            Ok(report) => {
                tracing::info!(?report, "sync daemon verify finished");
                *last_verify = Instant::now();
            }
            Err(e) => tracing::warn!(error = %e, "sync daemon verify failed"),
        }
    }
    Ok(())
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
