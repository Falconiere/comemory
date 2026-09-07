//! `comemory auth` — workspace-key device login against the cloud platform.
//!
//! Nested: `login` / `status` / `logout`. Logic lives in [`crate::cloud`];
//! this module owns clap + TTY/JSON rendering. CLI-only (no `/api/v1` route).

use std::io::Write as _;
use std::path::PathBuf;

use clap::{Args as ClapArgs, Subcommand};
use owo_colors::OwoColorize;
use serde::Serialize;

use crate::cloud::{self, StatusReport};
use crate::config::paths::{Paths, resolve_data_dir};
use crate::output::json;
use crate::prelude::*;

const EXAMPLES: &str = "\
Examples:
  # Device login (print code, approve in the console, mint cmk_)
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
    /// RFC 8628 device login; mint a workspace-bound `cmk_` into auth.json.
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
    workspace_id: &'a str,
    key_prefix: &'a str,
    secret: &'a str,
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
    cloud::save(&paths.auth_file(), &outcome.credentials)?;
    let creds = &outcome.credentials;
    if json_flag {
        return json::write(&LoginJson {
            authenticated: true,
            api_url: &creds.api_url,
            workspace_id: &creds.workspace_id,
            key_prefix: &creds.key_prefix,
            secret: &creds.secret,
        });
    }
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "{} logged in to workspace {} ({})",
        "\u{2713}".green(),
        creds.workspace_id.bold(),
        creds.key_prefix.dimmed()
    )?;
    writeln!(
        out,
        "  api {} · credentials {}",
        creds.api_url,
        paths.auth_file().display()
    )?;
    Ok(())
}

fn run_status(paths: &Paths, a: StatusArgs, json_flag: bool) -> Result<()> {
    let file = cloud::load(&paths.auth_file())?;
    let secret = cloud::effective_secret(file.as_ref())?;
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
    let mut report = cloud::workspace_status(&api_url, &secret)?;
    if report.key_prefix.is_none() {
        report.key_prefix = file.as_ref().map(|c| c.key_prefix.clone());
    }
    if report.workspace_id.is_none() {
        report.workspace_id = file.as_ref().map(|c| c.workspace_id.clone());
    }
    emit_status(json_flag, &report)
}

fn run_logout(paths: &Paths, json_flag: bool) -> Result<()> {
    cloud::clear(&paths.auth_file())?;
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
        workspace_id: None,
        workspace_name: None,
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
            "{} authenticated · workspace {} ({})",
            "\u{2713}".green(),
            report
                .workspace_name
                .as_deref()
                .or(report.workspace_id.as_deref())
                .unwrap_or("unknown")
                .bold(),
            report.key_prefix.as_deref().unwrap_or("cmk_????").dimmed()
        )?;
        if let Some(api) = &report.api_url {
            writeln!(out, "  api {api}")?;
        }
        if let (Some(id), Some(name)) = (&report.workspace_id, &report.workspace_name)
            && name != id
        {
            writeln!(out, "  id {id}")?;
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
