//! `comemory sync` — push/pull against the platform. CLI-only.
//!
//! Nested `daemon {install,uninstall,start,stop,status,run}` owns continuous
//! auto-sync. Flat `--action` still drives a one-shot manual sync.

use std::io::Write as _;
use std::path::PathBuf;

use clap::{Args as ClapArgs, Subcommand, ValueEnum};

use crate::cli::load_config;
use crate::cli::off_runtime::off_runtime;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::config::sync::apply_embed_model;
use crate::output::json;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::connection::open;
use crate::store::sync_state;
use crate::sync::auth_file::AuthFile;
use crate::sync::daemon::{self, DaemonStatus};
use crate::sync::{pull, push, verify};

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
    let auth = AuthFile::load(paths)?
        .ok_or_else(|| Error::Usage("not logged in — run `comemory auth login`".into()))?;
    let workspace = auth.workspace_id.clone();
    let mut conn = open(paths.db_path())?;
    apply_embed_model(&conn, &cfg.embed)?;

    match action {
        SyncAction::Status => emit_status(json_flag, &mut conn, &workspace),
        SyncAction::Verify => {
            let report = off_runtime(|| verify::verify_manifests(paths, &cfg, &mut conn, &auth))?;
            emit_verify(json_flag, &report)
        }
        SyncAction::Push => {
            let stats =
                off_runtime(|| push::run_push(paths, &cfg, &mut conn, &auth, allow_secret, 2000))?;
            emit_run(json_flag, &workspace, None, Some(&stats))
        }
        SyncAction::Pull => {
            let stats = off_runtime(|| pull::run_pull(paths, &cfg, &mut conn, &auth, 2000))?;
            emit_run(json_flag, &workspace, Some(&stats), None)
        }
        SyncAction::Run => {
            let (pull_stats, push_stats) = off_runtime(|| {
                let pulled = pull::run_pull(paths, &cfg, &mut conn, &auth, 2000)?;
                let pushed = push::run_push(paths, &cfg, &mut conn, &auth, allow_secret, 2000)?;
                Ok((pulled, pushed))
            })?;
            emit_run(json_flag, &workspace, Some(&pull_stats), Some(&push_stats))
        }
    }
}

fn emit_daemon_status(json_flag: bool, st: &DaemonStatus) -> Result<()> {
    if json_flag {
        json::write(st)?;
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(out, "platform: {}", st.platform)?;
        if let Some(path) = &st.unit_path {
            writeln!(out, "unit: {path}")?;
        }
        writeln!(out, "installed: {}", st.installed)?;
        writeln!(out, "running: {}", st.running)?;
        writeln!(out, "detail: {}", st.detail)?;
        if let Some(warn) = st.inactive_warning() {
            writeln!(out, "warning: {warn}")?;
        }
    }
    Ok(())
}

fn emit_status(json_flag: bool, conn: &mut Connection, workspace: &str) -> Result<()> {
    let row = sync_state::get(conn, workspace)?;
    let head = crate::store::sync_log::head_seq(conn)?;
    let (pushed, pulled, last_sync) = row.as_ref().map_or((0, 0, None), |r| {
        (r.pushed_seq, r.pulled_seq, r.last_sync_at.clone())
    });
    let daemon = match daemon::status() {
        Ok(st) => st,
        Err(_) => DaemonStatus {
            platform: "unknown",
            unit_path: None,
            installed: false,
            running: false,
            detail: "daemon status unavailable".into(),
        },
    };
    if json_flag {
        json::write(&serde_json::json!({
            "workspace": workspace,
            "pushed_seq": pushed,
            "pulled_seq": pulled,
            "head_seq": head,
            "last_sync_at": last_sync,
            "daemon": daemon,
        }))?;
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(out, "workspace: {workspace}")?;
        writeln!(out, "pushed_seq: {pushed}")?;
        writeln!(out, "pulled_seq: {pulled}")?;
        writeln!(out, "head_seq: {head}")?;
        writeln!(
            out,
            "daemon: installed={} running={} ({})",
            daemon.installed, daemon.running, daemon.detail
        )?;
        if let Some(warn) = daemon.inactive_warning() {
            writeln!(out, "warning: {warn}")?;
        }
    }
    Ok(())
}

fn emit_verify(json_flag: bool, report: &verify::VerifyReport) -> Result<()> {
    if json_flag {
        json::write(report)?;
    } else {
        let mut out = std::io::stdout().lock();
        if report.differing_buckets == 0 {
            let suffix = if report.repaired { " after repair" } else { "" };
            writeln!(
                out,
                "Manifests match{suffix} (head local={}, remote={})",
                report.local_head_seq, report.remote_head_seq
            )?;
        } else {
            writeln!(
                out,
                "{} bucket(s) still differ after repair (local head={}, remote head={})",
                report.differing_buckets, report.local_head_seq, report.remote_head_seq
            )?;
        }
    }
    Ok(())
}

fn emit_run(
    json_flag: bool,
    workspace: &str,
    pull_stats: Option<&pull::PullStats>,
    push_stats: Option<&push::PushStats>,
) -> Result<()> {
    if json_flag {
        json::write(&serde_json::json!({
            "workspace": workspace,
            "push": push_stats,
            "pull": pull_stats,
        }))?;
    } else {
        let mut out = std::io::stdout().lock();
        if let Some(p) = pull_stats {
            writeln!(
                out,
                "Pulled {} entries (seq {})",
                p.pulled, p.last_pulled_seq
            )?;
        }
        if let Some(p) = push_stats {
            writeln!(
                out,
                "Pushed {} entries (skipped personal={}, skip_repos={}, blocked_secrets={}, rejected_repo={})",
                p.pushed,
                p.skipped_personal,
                p.skipped_config,
                p.blocked_secrets,
                p.rejected_repo
            )?;
        }
    }
    Ok(())
}
