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

const EXAMPLES: &str = "\
Examples:
  # Log in (print code, approve in the console, mint an org cmk_)
  # and run the first sync before returning
  comemory auth login

  # Point at a non-prod API
  comemory auth login --api-url https://dev-api.comemory.io

  # Check the saved key against the platform
  comemory auth status

  # Forget local credentials (no remote revoke)
  comemory auth logout

  # Machine-readable
  comemory auth login --json
  comemory auth status --json";

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
    /// RFC 8628 device login; mint an organization `cmk_` into auth.json and
    /// run the first sync.
    Login(LoginArgs),
    /// Report whether local credentials still authenticate.
    Status(StatusArgs),
    /// Delete local auth.json (no remote revoke).
    Logout,
}

/// Flags for `comemory auth login`.
#[derive(ClapArgs, Debug)]
pub struct LoginArgs {
    /// Platform API base URL (overrides `COMEMORY_API` / default).
    #[arg(long, value_name = "URL")]
    pub api_url: Option<String>,
}

/// Flags for `comemory auth status`.
#[derive(ClapArgs, Debug)]
pub struct StatusArgs {
    /// Platform API base URL override (else auth.json / `COMEMORY_API`).
    #[arg(long, value_name = "URL")]
    pub api_url: Option<String>,
}

/// JSON envelope for a successful login (includes `secret` once for scripting).
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
    initial_sync: InitialSyncJson,
}

/// Outcome of the login-time first sync. Reported either way: a login whose
/// sync failed is still a usable login, and the user needs to know which.
#[derive(Serialize)]
struct InitialSyncJson {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pulled: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pushed: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// JSON envelope when logout succeeds.
#[derive(Serialize)]
struct LogoutJson {
    logged_out: bool,
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
    // A cached repo allowlist from before organization scoping is dead weight
    // and must not outlive the credential it was fetched for.
    auth_file::clear_stale_allowlist(paths)?;
    let creds = &outcome.credentials;

    // Best-effort by design: the credential is already on disk and useful, so
    // a network blip here must not leave the user logged out with no next
    // step. It runs inline rather than detached because the counts belong in
    // the report below, which a detached thread could not fill in.
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
            initial_sync: match &synced {
                Ok(stats) => InitialSyncJson {
                    ok: true,
                    pulled: Some(stats.pulled),
                    pushed: Some(stats.pushed),
                    error: None,
                },
                Err(e) => InitialSyncJson {
                    ok: false,
                    pulled: None,
                    pushed: None,
                    error: Some(e.to_string()),
                },
            },
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
    match &synced {
        Ok(stats) => writeln!(
            out,
            "  synced: pulled {} · pushed {}",
            stats.pulled, stats.pushed
        )?,
        Err(e) => {
            let mut err_out = std::io::stderr().lock();
            writeln!(
                err_out,
                "warning: first sync failed ({e}) — run `comemory sync` when the platform is reachable"
            )?;
        }
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

/// Display label for the organization a credential is scoped to: its name when
/// the platform supplied one, else the slug, else the raw id.
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

fn run_logout(paths: &Paths, json_flag: bool) -> Result<()> {
    AuthFile::clear(paths)?;
    if json_flag {
        return json::write(&LogoutJson { logged_out: true });
    }
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "{} logged out (local credentials removed)",
        "\u{2713}".green()
    )?;
    Ok(())
}

fn emit_logged_out(json_flag: bool) -> Result<()> {
    let report = StatusReport {
        authenticated: false,
        api_url: None,
        organization_id: None,
        organization_name: None,
        workspace_id: None,
        key_prefix: None,
    };
    emit_status(json_flag, &report)
}

fn emit_status(json_flag: bool, report: &StatusReport) -> Result<()> {
    if json_flag {
        json::write(report)?;
    } else if report.authenticated {
        let mut out = std::io::stdout().lock();
        writeln!(
            out,
            "{} authenticated · organization {} ({})",
            "\u{2713}".green(),
            report
                .organization_name
                .as_deref()
                .filter(|name| !name.is_empty())
                .or(report.organization_id.as_deref())
                .unwrap_or("unknown")
                .bold(),
            report.key_prefix.as_deref().unwrap_or("cmk_????").dimmed()
        )?;
        if let Some(api) = &report.api_url {
            writeln!(out, "  api {api}")?;
        }
        if let Some(workspace) = &report.workspace_id {
            writeln!(out, "  workspace {workspace}")?;
        }
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(out, "not logged in")?;
    }
    if report.authenticated {
        Ok(())
    } else {
        Err(Error::Unavailable("not logged in".into()))
    }
}
