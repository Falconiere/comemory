//! `comemory auth` — organization login against the cloud platform.
//!
//! Nested: `login` / `status` / `logout`. Logic lives in [`crate::cloud`] and
//! [`crate::sync::initial`]; this module owns clap + TTY/JSON rendering.
//! CLI-only (no `/api/v1` route).

use std::io::Write as _;
use std::path::PathBuf;

use crate::cli::auth_render::{
    DaemonLoginJson, LoginJson, emit_logged_out, emit_status, initial_sync_json, org_label,
    write_logout,
};
use crate::cli::load_config;
use crate::cli::off_runtime::off_runtime;
use crate::cloud;
use crate::config::env;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::output::json;
use crate::prelude::*;
use crate::sync::auth_file::{self, AuthFile};
use crate::sync::daemon;
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
    let api_url = cloud::resolve_api_url(a.api_url.as_deref())?;
    let mut progress = std::io::stderr().lock();
    let outcome = cloud::login(&api_url, &mut progress)?;
    drop(progress);
    outcome.credentials.save(paths)?;
    auth_file::clear_stale_allowlist(paths)?;
    let creds = &outcome.credentials;

    let daemon_json = if a.daemon {
        daemon::install_and_start_best_effort(paths);
        DaemonLoginJson {
            skipped: false,
            running: daemon::status().ok().map(|s| s.running),
        }
    } else {
        DaemonLoginJson {
            skipped: true,
            running: None,
        }
    };

    // Best-effort: credential is already on disk; sync counts belong in the report.
    let cfg = load_config(paths)?;
    let synced = off_runtime(|| crate::sync::initial::run_initial_sync(paths, &cfg, creds));
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
    let file = AuthFile::load(paths)?;
    let secret = match &file {
        Some(creds) => Some(creds.effective_secret()),
        None => env::api_key_override(),
    };
    let Some(secret) = secret else {
        return emit_logged_out(json_flag);
    };
    let api_url = if let Some(raw) = a.api_url.as_deref() {
        cloud::resolve_api_url(Some(raw))?
    } else if let Some(creds) = &file {
        creds.api_url.clone()
    } else {
        cloud::resolve_api_url(None)?
    };
    let mut report = cloud::org_status(&api_url, &secret, file.as_ref())?;
    if report.key_prefix.is_none() {
        report.key_prefix = file.as_ref().map(|c| c.key_prefix.clone());
    }
    emit_status(json_flag, &report)
}

fn run_logout(paths: &Paths, json_flag: bool) -> Result<()> {
    // Stop while credentials still exist so a failing stop does not leave the
    // daemon racing against a deleted auth.json mid-clear.
    daemon::stop_best_effort();
    let daemon_stopped = daemon::status().map_or(true, |s| !s.running);
    AuthFile::clear(paths)?;
    write_logout(json_flag, daemon_stopped)
}
