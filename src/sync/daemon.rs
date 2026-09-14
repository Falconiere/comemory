//! User-level OS daemon that periodically pull+push syncs.
//!
//! Continuous auto-sync lives here — not in `save` / `context`. Login installs
//! and starts the unit by default; logout stops it (unit stays installed).
//! Manual `comemory sync` still works with the daemon stopped.
//!
//! Unit install/start/stop/status live in [`crate::sync::daemon_unit`].

use std::thread;
use std::time::{Duration, Instant};

use crate::config::sync::apply_embed_model;
use crate::config::{Config, Paths};
use crate::prelude::*;
use crate::store::connection::open;
use crate::sync::AuthFile;
use crate::sync::daemon_unit;
use crate::sync::{pull, push, verify};

pub use daemon_unit::{
    DAEMON_LABEL, DaemonStatus, SYSTEMD_UNIT, install, launch_agent_plist,
    render_launch_agent_plist, render_systemd_unit, start, status, stop, systemd_user_unit,
    uninstall,
};

/// Page size for each daemon cycle (same as manual `comemory sync`).
const CYCLE_LIMIT: usize = 2000;

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
        thread::sleep(interval);
    }
}

fn run_one_cycle(
    paths: &Paths,
    cfg: &Config,
    last_verify: &mut Instant,
    verify_every: Duration,
) -> Result<()> {
    let Some(auth) = AuthFile::load_usable(paths)? else {
        tracing::debug!("sync daemon: not logged in — skipping cycle");
        return Ok(());
    };
    let mut conn = open(paths.db_path())?;
    apply_embed_model(&conn, &cfg.embed)?;
    if let Err(e) = pull::run_pull(paths, cfg, &mut conn, &auth, CYCLE_LIMIT) {
        tracing::warn!(error = %e, "sync daemon pull failed");
    }
    if let Err(e) = push::run_push(paths, cfg, &mut conn, &auth, None, CYCLE_LIMIT) {
        tracing::warn!(error = %e, "sync daemon push failed");
    }
    if last_verify.elapsed() >= verify_every {
        match verify::verify_manifests(paths, cfg, &mut conn, &auth) {
            Ok(report) => {
                tracing::info!(
                    differing = report.differing_buckets,
                    repaired = report.repaired,
                    "sync daemon verify finished"
                );
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
    if std::env::var_os("COMEMORY_SYNC_DAEMON").is_some_and(|v| v == "0") {
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
    stop();
}

#[cfg(test)]
#[path = "tests/daemon.rs"]
mod tests;
