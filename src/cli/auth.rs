//! `comemory auth` — organization login against the cloud platform.
//!
//! Nested: `login` / `status` / `logout`. The sequences live in
//! [`crate::domains::sync::login`]; this module owns clap, the progress
//! destination and TTY/JSON rendering. CLI-only (no `/api/v1` route).

use std::io::Write as _;
use std::path::PathBuf;

use crate::cli::auth_render::{
    DaemonLoginJson, LoginJson, emit_logged_out, emit_status, initial_sync_json, org_label,
    write_logout,
};
use crate::cli::load_config;
use crate::cli::off_runtime::off_runtime;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::domains::sync::daemon;
use crate::domains::sync::login;
use crate::output::json;
use crate::prelude::*;
use clap::{Args as ClapArgs, Subcommand};
use owo_colors::OwoColorize;

const EXAMPLES: &str = "\
Examples:
  comemory auth login
  comemory auth login --daemon
  comemory auth login --api-url https://dev-api.comemory.io
  comemory auth status
  comemory auth logout";

/// Arguments to `comemory auth` (nested subcommand required).
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// `login` / `status` / `logout`.
    #[command(subcommand)]
    pub cmd: AuthCmd,
}

/// Nested `comemory auth <subcommand>`.
#[derive(Subcommand, Debug)]
pub enum AuthCmd {
    /// Device login, mint org key, optional daemon install, full first sync.
    Login(LoginArgs),
    /// Report whether local credentials still authenticate.
    Status(StatusArgs),
    /// Delete local auth.json and stop the sync daemon (no remote revoke).
    Logout,
}

/// Flags for `comemory auth login`.
#[derive(ClapArgs, Debug)]
pub struct LoginArgs {
    /// Platform API base URL (overrides `COMEMORY_API` / default).
    #[arg(long, value_name = "URL")]
    pub api_url: Option<String>,
    /// Also install and start the user-level sync daemon.
    ///
    /// Off by default since the 2026-09-14 sync design: a save pushes inline
    /// and `comemory watch` covers the pull direction, so a resident process
    /// is for headless hosts rather than the common case. Replaces the old
    /// `--no-daemon`, which opted out of an install that no longer happens.
    #[arg(long, default_value_t = false)]
    pub daemon: bool,
}

/// Flags for `comemory auth status`.
#[derive(ClapArgs, Debug)]
pub struct StatusArgs {
    /// Platform API base URL override (else auth.json / `COMEMORY_API`).
    #[arg(long, value_name = "URL")]
    pub api_url: Option<String>,
}

/// Dispatch nested auth subcommands.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    match a.cmd {
        AuthCmd::Login(login) => run_login(&paths, login, json_flag),
        AuthCmd::Status(status) => run_status(&paths, status, json_flag),
        AuthCmd::Logout => run_logout(&paths, json_flag),
    }
}

fn run_login(paths: &Paths, a: LoginArgs, json_flag: bool) -> Result<()> {
    let mut progress = std::io::stderr().lock();
    let established = login::establish(paths, a.api_url.as_deref(), a.daemon, &mut progress)?;
    drop(progress);
    let creds = &established.credentials;
    let daemon_json = DaemonLoginJson {
        skipped: established.daemon_skipped,
        running: established.daemon_running,
    };

    // Best-effort: credential is already on disk; sync counts belong in the report.
    let cfg = load_config(paths)?;
    let synced =
        off_runtime(|| crate::domains::sync::initial::run_initial_sync(paths, &cfg, creds));
    if let Err(e) = &synced {
        tracing::warn!(error = %e, "first sync after login failed");
    }

    if json_flag {
        return json::write(&LoginJson {
            authenticated: true,
            api_url: &creds.api_url,
            organization_id: &creds.organization_id,
            organization_slug: &creds.organization_slug,
            organization_name: &creds.organization_name,
            workspace_id: &creds.workspace_id,
            key_prefix: &creds.key_prefix,
            secret: &creds.secret,
            daemon: daemon_json,
            initial_sync: initial_sync_json(&synced),
        });
    }
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "{} logged in to {} ({})",
        "\u{2713}".green(),
        org_label(creds).bold(),
        creds.key_prefix.dimmed()
    )?;
    writeln!(
        out,
        "  api {} · credentials {}",
        creds.api_url,
        paths.auth_file().display()
    )?;
    if a.daemon {
        if let Ok(st) = daemon::status() {
            writeln!(out, "  daemon: {}", st.detail)?;
        }
    } else {
        writeln!(
            out,
            "  daemon: not installed (saves push inline; `comemory watch` for live pulls)"
        )?;
    }
    match &synced {
        Ok(stats) => {
            writeln!(
                out,
                "  synced: pulled {} · pushed {} · skip_repos={}",
                stats.pulled, stats.pushed, stats.skipped_config
            )?;
            writeln!(
                out,
                "  {}",
                crate::cli::sync_render::code_summary_line(stats)
            )?;
        }
        Err(e) => writeln!(
            std::io::stderr().lock(),
            "warning: first sync failed ({e}) — run `comemory sync` when the platform is reachable"
        )?,
    }
    Ok(())
}

fn run_status(paths: &Paths, a: StatusArgs, json_flag: bool) -> Result<()> {
    match login::status(paths, a.api_url.as_deref())? {
        Some(report) => emit_status(json_flag, &report),
        None => emit_logged_out(json_flag),
    }
}

fn run_logout(paths: &Paths, json_flag: bool) -> Result<()> {
    write_logout(json_flag, login::logout(paths)?)
}
