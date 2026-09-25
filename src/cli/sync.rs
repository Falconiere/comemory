//! `comemory sync` — push/pull against the platform. CLI-only.
//!
//! Nested `daemon {install,uninstall,start,stop,status,run}` owns continuous
//! auto-sync. Flat `--action` still drives a one-shot manual sync. `run` and
//! `push` push the code index after the memories (`domains::sync::code`); rendering
//! lives in `cli::sync_render`. `--action auto` (what git and agent hooks fire)
//! is dispatched to `cli::sync_auto` before any login is required.

use std::io::Write as _;
use std::path::PathBuf;

use clap::{Args as ClapArgs, Subcommand, ValueEnum};

use crate::cli::load_config;
use crate::cli::off_runtime::off_runtime;
use crate::cli::output::json;
use crate::cli::sync_auto;
use crate::cli::sync_render::{emit_daemon_status, emit_run, emit_status, emit_verify};
use crate::config::paths::{Paths, resolve_data_dir};
use crate::domains::sync::auto::hold_pass_lock;
use crate::domains::sync::daemon;
use crate::domains::sync::drain::session::Legs;
use crate::domains::sync::manual;
use crate::domains::sync::verify;
use crate::prelude::*;

const EXAMPLES: &str = "\
Examples:
  comemory sync
  comemory sync --action push
  comemory sync --action status --json
  comemory sync --action verify
  comemory sync --allow-secret deadbeef
  comemory sync --action auto --path /path/to/repo
  comemory sync daemon status
  comemory sync daemon install
  comemory sync daemon start
  comemory sync daemon stop
  comemory sync daemon uninstall
  comemory sync daemon run";

/// Sync mode — replaces four separate bool flags (clippy `struct_excessive_bools`).
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum SyncAction {
    /// Pull, then push, until the upstream is drained (default).
    #[default]
    Run,
    /// Push local changes only (`--push-only` alias).
    #[value(name = "push", alias = "push-only")]
    Push,
    /// Pull remote changes only (`--pull-only` alias).
    #[value(name = "pull", alias = "pull-only")]
    Pull,
    /// Compare local/remote manifests (per kind on `replica-v1`) and repair
    /// differing buckets.
    Verify,
    /// Print sync cursors and the key's `exchange` state.
    Status,
    /// The unattended pass git hooks and agent hooks fire: index `--path`,
    /// refresh every stale hooked repo, then drain both ways when logged in.
    /// Needs no login; prints nothing without `--json`; coalesces with a
    /// pass that is already queued.
    Auto,
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
    /// Operation: `run` (default), `push`, `pull`, `verify`, `status`, or
    /// `auto`.
    #[arg(long, value_enum, default_value_t = SyncAction::Run)]
    pub action: SyncAction,
    /// With `--action auto` only: the checkout a git hook fired in, indexed
    /// first under its main worktree's label.
    #[arg(long, value_name = "CHECKOUT")]
    pub path: Option<PathBuf>,
    /// Record a secret-scan override for one memory id before push.
    #[arg(long, value_name = "ID")]
    pub allow_secret: Option<String>,
}

/// Run push/pull/status/verify/auto or a daemon subcommand.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    if let Some(SyncCmd::Daemon(d)) = a.cmd {
        if matches!(d.cmd, DaemonCmd::Run) {
            // Foreground: the supervisor's entry; it returns once stopped.
            return daemon::run_foreground(&paths).await;
        }
        return run_daemon(&paths, d.cmd, json_flag);
    }
    if a.path.is_some() && !matches!(a.action, SyncAction::Auto) {
        return Err(Error::Usage(
            "--path is only accepted with --action auto".into(),
        ));
    }
    run_sync(&paths, &a, json_flag)
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
        DaemonCmd::Run => Err(Error::Other(
            "`sync daemon run` is dispatched before this match".into(),
        )),
    }
}

/// Run one `--action`. `auto` needs no credential, so it runs before any
/// session exists; every other action opens the session it needs first.
fn run_sync(paths: &Paths, a: &Args, json_flag: bool) -> Result<()> {
    let cfg = load_config(paths)?;
    let allow_secret = a.allow_secret.as_deref();
    let with_session = |act: &dyn Fn(&mut manual::Session) -> Result<()>| {
        act(&mut manual::open_session(paths, &cfg)?)
    };
    let exchange = |legs: Legs| {
        with_session(&|s| {
            let stats = off_runtime(|| manual::run(paths, &cfg, s, allow_secret, legs))?;
            emit_run(json_flag, &s.auth.workspace_id, &stats)?;
            // The report is out; a run that ended on the network still fails.
            stats
                .error
                .clone()
                .map_or(Ok(()), |e| Err(Error::Unavailable(e)))
        })
    };
    match a.action {
        SyncAction::Auto => sync_auto::run(paths, &cfg, a.path.as_deref(), json_flag),
        SyncAction::Status => with_session(&|s| emit_status(json_flag, &mut s.conn, &s.auth)),
        SyncAction::Verify => with_session(&|s| {
            let report = off_runtime(|| {
                let _pass = hold_pass_lock(paths)?;
                verify::verify(paths, &cfg, &mut s.conn, &s.auth)
            })?;
            emit_verify(json_flag, &report)
        }),
        SyncAction::Push => exchange(Legs::Push),
        SyncAction::Pull => exchange(Legs::Pull),
        SyncAction::Run => exchange(Legs::Both),
    }
}
