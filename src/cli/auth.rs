//! `comemory auth` — device login against the cloud platform.
//!
//! Nested: `login` / `status` / `logout`. Logic lives in [`crate::cloud`];
//! this module owns clap + TTY/JSON rendering. CLI-only (no `/api/v1` route).

use std::io::Write as _;
use std::path::PathBuf;

use clap::{Args as ClapArgs, Subcommand};
use owo_colors::OwoColorize;
use serde::Serialize;

use crate::cloud::{self, StatusReport};
use crate::config::env;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::output::json;
use crate::prelude::*;
use crate::sync::auth_file::AuthFile;

const EXAMPLES: &str = "\
Examples:
  # Device login (print code, approve in the console, mint cmk_)
  comemory auth login

  # Point at a non-prod API
  comemory auth login --api-url https://dev-api.comemory.io

  # Label this machine (default: hostname)
  comemory auth login --device-name laptop

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
    /// RFC 8628 device login; mint a device `cmk_` into auth.json.
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
    /// Label stored with the device key (default: this machine's hostname).
    #[arg(long, value_name = "NAME")]
    pub device_name: Option<String>,
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
    personal_workspace_id: &'a str,
    device_name: &'a str,
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
    let device_name = a.device_name.unwrap_or_else(default_device_name);
    let mut progress = std::io::stderr().lock();
    let outcome = cloud::login(&api_url, &device_name, &mut progress)?;
    outcome.credentials.save(paths)?;
    let creds = &outcome.credentials;
    if json_flag {
        return json::write(&LoginJson {
            authenticated: true,
            api_url: &creds.api_url,
            personal_workspace_id: &creds.personal_workspace_id,
            device_name: &creds.device_name,
            key_prefix: &creds.key_prefix,
            secret: &creds.secret,
        });
    }
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "{} logged in as device {} ({})",
        "\u{2713}".green(),
        creds.device_name.bold(),
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
    let personal_id = file.as_ref().map(|c| c.personal_workspace_id.clone());
    let mut report = cloud::workspace_status(&api_url, &secret, personal_id.as_deref())?;
    if report.key_prefix.is_none() {
        report.key_prefix = file.as_ref().map(|c| c.key_prefix.clone());
    }
    emit_status(json_flag, &report)
}

/// Hostname when the platform exposes one, else a stable fallback label.
fn default_device_name() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "comemory-cli".to_string())
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
