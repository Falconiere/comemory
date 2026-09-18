//! `comemory sync` — push/pull against the platform. CLI-only.
//!
//! Nested `daemon {install,uninstall,start,stop,status,run}` owns continuous
//! auto-sync. Flat `--action` still drives a one-shot manual sync. `run` and
//! `push` push the code index after the memories (`domains::sync::code`); rendering
//! lives in `cli::sync_render`.

use std::io::Write as _;
use std::path::PathBuf;

use clap::{Args as ClapArgs, Subcommand, ValueEnum};

use crate::cli::load_config;
use crate::cli::off_runtime::off_runtime;
use crate::cli::output::json;
use crate::cli::sync_render::{emit_daemon_status, emit_run, emit_status, emit_verify};
use crate::config::paths::{Paths, resolve_data_dir};
use crate::domains::sync::daemon;
use crate::domains::sync::manual::{self, RUN_LIMIT};
use crate::domains::sync::verify;
use crate::prelude::*;

const EXAMPLES: &str = "\
Examples:
  comemory sync
  comemory sync --action push
  comemory sync --action status --json
  comemory sync --action verify
  comemory sync --allow-secret deadbeef
  comemory sync daemon status
  comemory sync daemon install
  comemory sync daemon start
  comemory sync daemon stop
  comemory sync daemon uninstall
  comemory sync daemon run";

/// Sync mode — replaces four separate bool flags (clippy `struct_excessive_bools`).
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum SyncAction {
    /// Push then pull (default).
    #[default]
    Run,
    /// Push local changes only (`--push-only` alias).
    #[value(name = "push", alias = "push-only")]
    Push,
    /// Pull remote changes only (`--pull-only` alias).
    #[value(name = "pull", alias = "pull-only")]
    Pull,
    /// Compare local/remote manifests and repair differing buckets (AC-9).
    Verify,
    /// Print sync cursors.
    Status,
}

/// Nested `comemory sync <subcommand>` (today: `daemon`).
#[derive(Subcommand, Debug)]
pub enum SyncCmd {
    /// Manage the user-level auto-sync daemon (launchd / systemd --user).
    Daemon(DaemonArgs),
}

/// `comemory sync daemon` args.
#[derive(ClapArgs, Debug)]
pub struct DaemonArgs {
    /// install / uninstall / start / stop / status / run.
    #[command(subcommand)]
    pub cmd: DaemonCmd,
}

/// Nested daemon actions.
#[derive(Subcommand, Debug)]
pub enum DaemonCmd {
    /// Write the LaunchAgent / systemd user unit.
    Install,
    /// Remove the unit and stop it.
    Uninstall,
    /// Start (or kickstart) the installed unit.
    Start,
    /// Stop the daemon; leave the unit installed.
    Stop,
    /// Report installed / running.
    Status,
    /// Foreground loop (what the supervisor runs).
    Run,
}

/// Arguments to `comemory sync`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Nested subcommand (`daemon …`). When absent, `--action` runs.
    #[command(subcommand)]
    pub cmd: Option<SyncCmd>,
    /// Operation: `run` (default), `push`, `pull`, `verify`, or `status`.
    #[arg(long, value_enum, default_value_t = SyncAction::Run)]
    pub action: SyncAction,
    /// Record a secret-scan override for one memory id before push.
    #[arg(long, value_name = "ID")]
    pub allow_secret: Option<String>,
}

/// Run push/pull/status/verify or a daemon subcommand.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    if let Some(SyncCmd::Daemon(d)) = a.cmd {
        return run_daemon(&paths, d.cmd, json_flag);
    }
    run_sync(&paths, a.action, a.allow_secret.as_deref(), json_flag)
}

fn run_daemon(paths: &Paths, cmd: DaemonCmd, json_flag: bool) -> Result<()> {
    match cmd {
        DaemonCmd::Install => {
            let path = daemon::install(paths)?;
            if json_flag {
                return json::write(&serde_json::json!({
                    "installed": true,
                    "unit_path": path.display().to_string(),
                }));
            }
            let mut out = std::io::stdout().lock();
            writeln!(out, "installed {}", path.display())?;
            Ok(())
        }
        DaemonCmd::Uninstall => {
            daemon::uninstall()?;
            if json_flag {
                return json::write(&serde_json::json!({ "uninstalled": true }));
            }
            let mut out = std::io::stdout().lock();
            writeln!(out, "uninstalled sync daemon")?;
            Ok(())
        }
        DaemonCmd::Start => {
            daemon::start()?;
            if json_flag {
                return json::write(&serde_json::json!({ "started": true }));
            }
            let mut out = std::io::stdout().lock();
            writeln!(out, "started sync daemon")?;
            Ok(())
        }
        DaemonCmd::Stop => {
            daemon::stop();
            if json_flag {
                return json::write(&serde_json::json!({ "stopped": true }));
            }
            let mut out = std::io::stdout().lock();
            writeln!(out, "stopped sync daemon")?;
            Ok(())
        }
        DaemonCmd::Status => {
            let st = daemon::status()?;
            emit_daemon_status(json_flag, &st)
        }
        DaemonCmd::Run => {
            // Foreground — never returns Ok on the happy path; Err propagates as the arm value.
            off_runtime(|| daemon::run_foreground(paths))
        }
    }
}

fn run_sync(
    paths: &Paths,
    action: SyncAction,
    allow_secret: Option<&str>,
    json_flag: bool,
) -> Result<()> {
    let cfg = load_config(paths)?;
    let mut session = manual::open_session(paths, &cfg)?;
    let workspace = session.auth.workspace_id.clone();

    let stats = match action {
        SyncAction::Status => return emit_status(json_flag, &mut session.conn, &workspace),
        SyncAction::Verify => {
            let report = off_runtime(|| {
                verify::verify_manifests(paths, &cfg, &mut session.conn, &session.auth)
            })?;
            return emit_verify(json_flag, &report);
        }
        SyncAction::Push => {
            off_runtime(|| manual::push_only(paths, &cfg, &mut session, allow_secret, RUN_LIMIT))?
        }
        SyncAction::Pull => {
            off_runtime(|| manual::pull_only(paths, &cfg, &mut session, RUN_LIMIT))?
        }
        SyncAction::Run => {
            off_runtime(|| manual::run_all(paths, &cfg, &mut session, allow_secret, RUN_LIMIT))?
        }
    };
    emit_run(
        json_flag,
        &workspace,
        stats.pull.as_ref(),
        stats.push.as_ref(),
        stats.code.as_ref(),
    )
}
