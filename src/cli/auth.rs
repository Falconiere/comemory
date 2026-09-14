//! `comemory auth` — organization login against the cloud platform.
//!
//! Nested: `login` / `status` / `logout`. Logic lives in [`crate::cloud`] and
//! [`crate::sync::initial`]; this module owns clap + TTY/JSON rendering.
//! CLI-only (no `/api/v1` route).

use std::io::Write as _;
use std::path::PathBuf;

use clap::{Args as ClapArgs, Subcommand};
use owo_colors::OwoColorize;
use serde::Serialize;

use crate::cli::load_config;
use crate::cli::off_runtime::off_runtime;
use crate::cloud::{self, StatusReport};
use crate::config::env;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::output::json;
use crate::prelude::*;
use crate::sync::auth_file::{self, AuthFile};
use crate::sync::daemon;

const EXAMPLES: &str = "\
Examples:
  comemory auth login
  comemory auth login --no-daemon
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
    /// Do not install or start the user-level sync daemon.
    #[arg(long, default_value_t = false)]
    pub no_daemon: bool,
}

/// Flags for `comemory auth status`.
#[derive(ClapArgs, Debug)]
pub struct StatusArgs {
    /// Platform API base URL override (else auth.json / `COMEMORY_API`).
    #[arg(long, value_name = "URL")]
    pub api_url: Option<String>,
}

#[derive(Serialize)]
struct LoginJson<'a> {
    authenticated: bool,
    api_url: &'a str,
    organization_id: &'a str,
    organization_slug: &'a str,
    organization_name: &'a str,
    workspace_id: &'a str,
    key_prefix: &'a str,
    secret: &'a str,
    daemon: DaemonLoginJson,
    initial_sync: InitialSyncJson,
}

#[derive(Serialize)]
struct DaemonLoginJson {
    skipped: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    running: Option<bool>,
}

#[derive(Serialize)]
struct InitialSyncJson {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pulled: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pushed: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    skipped_personal: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    skipped_config: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct LogoutJson {
    logged_out: bool,
    daemon_stopped: bool,
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

    let daemon_json = if a.no_daemon {
        DaemonLoginJson {
            skipped: true,
            running: None,
        }
    } else {
        daemon::install_and_start_best_effort(paths);
        DaemonLoginJson {
            skipped: false,
            running: daemon::status().ok().map(|s| s.running),
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
    if a.no_daemon {
        writeln!(out, "  daemon: skipped (--no-daemon)")?;
    } else if let Ok(st) = daemon::status() {
        writeln!(out, "  daemon: {}", st.detail)?;
    }
    match &synced {
        Ok(stats) => writeln!(
            out,
            "  synced: pulled {} · pushed {} · skipped personal={} · skip_repos={}",
            stats.pulled, stats.pushed, stats.skipped_personal, stats.skipped_config
        )?,
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

/// Display label for the organization a credential is scoped to.
fn org_label(creds: &AuthFile) -> &str {
    for candidate in [
        creds.organization_name.as_str(),
        creds.organization_slug.as_str(),
        creds.organization_id.as_str(),
    ] {
        if !candidate.is_empty() {
            return candidate;
        }
    }
    "unknown organization"
}

fn initial_sync_json(synced: &Result<crate::sync::InitialSyncStats>) -> InitialSyncJson {
    match synced {
        Ok(s) => InitialSyncJson {
            ok: true,
            pulled: Some(s.pulled),
            pushed: Some(s.pushed),
            skipped_personal: Some(s.skipped_personal),
            skipped_config: Some(s.skipped_config),
            error: None,
        },
        Err(e) => InitialSyncJson {
            ok: false,
            pulled: None,
            pushed: None,
            skipped_personal: None,
            skipped_config: None,
            error: Some(e.to_string()),
        },
    }
}

fn run_logout(paths: &Paths, json_flag: bool) -> Result<()> {
    daemon::stop_best_effort();
    AuthFile::clear(paths)?;
    if json_flag {
        return json::write(&LogoutJson {
            logged_out: true,
            daemon_stopped: true,
        });
    }
    writeln!(
        std::io::stdout().lock(),
        "{} logged out (local credentials removed; sync daemon stopped)",
        "\u{2713}".green()
    )?;
    Ok(())
}

fn emit_logged_out(json_flag: bool) -> Result<()> {
    emit_status(
        json_flag,
        &StatusReport {
            authenticated: false,
            api_url: None,
            organization_id: None,
            organization_name: None,
            workspace_id: None,
            key_prefix: None,
        },
    )
}

fn emit_status(json_flag: bool, report: &StatusReport) -> Result<()> {
    let daemon_st = report
        .authenticated
        .then(|| daemon::status().ok())
        .flatten();
    if json_flag {
        json::write(&serde_json::json!({
            "authenticated": report.authenticated,
            "api_url": report.api_url,
            "organization_id": report.organization_id,
            "organization_name": report.organization_name,
            "workspace_id": report.workspace_id,
            "key_prefix": report.key_prefix,
            "daemon": daemon_st,
        }))?;
    } else if report.authenticated {
        let mut out = std::io::stdout().lock();
        let org = report
            .organization_name
            .as_deref()
            .filter(|n| !n.is_empty())
            .or(report.organization_id.as_deref())
            .unwrap_or("unknown");
        writeln!(
            out,
            "{} authenticated · organization {} ({})",
            "\u{2713}".green(),
            org.bold(),
            report.key_prefix.as_deref().unwrap_or("cmk_????").dimmed()
        )?;
        if let Some(api) = &report.api_url {
            writeln!(out, "  api {api}")?;
        }
        if let Some(workspace) = &report.workspace_id {
            writeln!(out, "  workspace {workspace}")?;
        }
        if let Some(st) = &daemon_st {
            writeln!(
                out,
                "  daemon: installed={} running={}",
                st.installed, st.running
            )?;
            if let Some(warn) = st.inactive_warning() {
                writeln!(out, "  warning: {warn}")?;
            }
        }
    } else {
        writeln!(std::io::stdout().lock(), "not logged in")?;
    }
    if report.authenticated {
        Ok(())
    } else {
        Err(Error::Unavailable("not logged in".into()))
    }
}
