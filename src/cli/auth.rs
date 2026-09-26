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
use crate::cli::output::json;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::domains::sync::initial::InitialSyncStats;
use crate::domains::sync::login::{self, Established};
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
    /// Deprecated, no-op: the sync daemon (#257) is always ensured by
    /// preflight now, so there is nothing left to opt into. Kept parseable
    /// (with a warning) rather than refused, unlike the removed
    /// `--no-daemon`, so a script that already passes it keeps working.
    #[arg(long, hide = true, default_value_t = false)]
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
    if a.daemon {
        writeln!(
            std::io::stderr().lock(),
            "warning: --daemon is deprecated and has no effect — the sync daemon is always ensured"
        )?;
    }
    let mut progress = std::io::stderr().lock();
    let cfg = load_config(paths)?;
    let established = login::establish((paths, &cfg), a.api_url.as_deref(), &mut progress)?;
    drop(progress);
    let creds = &established.credentials;
    let daemon_json = DaemonLoginJson {
        running: established.daemon_running,
        skipped: false,
        instance: established.daemon_instance.clone(),
    };

    // Best-effort: credential is already on disk; sync counts belong in the report.
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
    write_login_text(paths, &established, &synced)
}

/// The human-readable login report: who is logged in, where the
/// credential lives, the daemon, and the first sync's outcome.
fn write_login_text(
    paths: &Paths,
    established: &Established,
    synced: &Result<InitialSyncStats>,
) -> Result<()> {
    let creds = &established.credentials;
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
    writeln!(
        out,
        "  daemon: running={} instance={}",
        established.daemon_running,
        established.daemon_instance.as_deref().unwrap_or("-")
    )?;
    match synced {
        Ok(stats) => {
            writeln!(
                out,
                "  synced: pulled {} · pushed {} · skip_repos={} · blocked_repo={}",
                stats.pulled, stats.pushed, stats.skipped_config, stats.blocked_repo
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
        Some(report) => emit_status(json_flag, paths, &report),
        None => emit_logged_out(json_flag, paths),
    }
}

fn run_logout(paths: &Paths, json_flag: bool) -> Result<()> {
    let cfg = load_config(paths).unwrap_or_else(|error| {
        tracing::warn!(%error, "logout: unreadable config; using defaults");
        crate::config::Config::defaults()
    });
    write_logout(json_flag, &login::logout(paths, &cfg)?)
}
