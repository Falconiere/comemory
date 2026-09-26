//! JSON shapes and TTY rendering for `comemory auth`.

use std::io::Write as _;

use owo_colors::OwoColorize;
use serde::Serialize;

use crate::cli::output::json;
use crate::config::Paths;
use crate::domains::sync::auth_file::AuthFile;
use crate::domains::sync::cloud::StatusReport;
use crate::domains::sync::daemon::status_view;
use crate::domains::sync::login::{InFlight, LoggedOut};
use crate::prelude::*;

/// `comemory auth login --json`'s report.
#[derive(Serialize)]
pub(crate) struct LoginJson<'a> {
    pub authenticated: bool,
    pub api_url: &'a str,
    pub organization_id: &'a str,
    pub organization_slug: &'a str,
    pub organization_name: &'a str,
    pub workspace_id: &'a str,
    pub key_prefix: &'a str,
    pub secret: &'a str,
    pub daemon: DaemonLoginJson,
    pub initial_sync: InitialSyncJson,
}

/// A verified coordinator's own record of `skipped` is always `false` now
/// (#257): every login is preceded by preflight ensuring one. The field
/// stays for schema stability with the pre-#257 shape.
#[derive(Serialize)]
pub(crate) struct DaemonLoginJson {
    pub running: bool,
    pub skipped: bool,
    pub instance: Option<String>,
}

/// The login report's first-sync outcome.
#[derive(Serialize)]
pub(crate) struct InitialSyncJson {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pulled: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pushed: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped_config: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_repo: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<crate::domains::sync::code::CodePushStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `comemory auth logout --json`'s report.
#[derive(Serialize)]
pub(crate) struct LogoutJson {
    pub logged_out: bool,
    pub daemon: LogoutDaemonJson,
}

/// The coordinator's state a logout observed.
#[derive(Serialize)]
pub(crate) struct LogoutDaemonJson {
    /// Whether a verified coordinator still answers — logout never stops it
    /// (D11), only its auth view changes.
    pub running: bool,
    /// What the wait for a pass already holding `sync.lock` found.
    pub in_flight: InFlight,
}

/// Display label for the organization a credential is scoped to.
pub(crate) fn org_label(creds: &AuthFile) -> &str {
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

pub(crate) fn initial_sync_json(
    synced: &Result<crate::domains::sync::initial::InitialSyncStats>,
) -> InitialSyncJson {
    match synced {
        Ok(s) => InitialSyncJson {
            ok: true,
            pulled: Some(s.pulled),
            pushed: Some(s.pushed),
            skipped_config: Some(s.skipped_config),
            blocked_repo: Some(s.blocked_repo),
            code: Some(s.code.clone()),
            code_error: s.code_error.clone(),
            error: None,
        },
        Err(e) => InitialSyncJson {
            ok: false,
            pulled: None,
            pushed: None,
            skipped_config: None,
            blocked_repo: None,
            code: None,
            code_error: None,
            error: Some(e.to_string()),
        },
    }
}

pub(crate) fn emit_logged_out(json_flag: bool, paths: &Paths) -> Result<()> {
    emit_status(
        json_flag,
        paths,
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

pub(crate) fn emit_status(json_flag: bool, paths: &Paths, report: &StatusReport) -> Result<()> {
    // Read regardless of `authenticated`: the coordinator is required and
    // healthy whether or not this machine is logged in (AC-4).
    let view = status_view::view(paths).ok();
    if json_flag {
        json::write(&serde_json::json!({
            "authenticated": report.authenticated,
            "api_url": report.api_url,
            "organization_id": report.organization_id,
            "organization_name": report.organization_name,
            "workspace_id": report.workspace_id,
            "key_prefix": report.key_prefix,
            "daemon": view,
        }))?;
    } else {
        write_status_text(report, view.as_ref())?;
    }
    if report.authenticated {
        Ok(())
    } else {
        Err(Error::Unavailable("not logged in".into()))
    }
}

/// The TTY half of [`emit_status`], split out to keep it under the
/// function-length ceiling.
fn write_status_text(report: &StatusReport, view: Option<&status_view::StatusView>) -> Result<()> {
    let mut out = std::io::stdout().lock();
    if !report.authenticated {
        writeln!(out, "not logged in")?;
        return Ok(());
    }
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
    if let Some(view) = view {
        writeln!(out, "  daemon: {}", view.detail)?;
        if view.version_matches == Some(false) {
            writeln!(
                out,
                "  warning: coordinator version differs from this binary"
            )?;
        }
    }
    Ok(())
}

pub(crate) fn write_logout(json_flag: bool, out: &LoggedOut) -> Result<()> {
    if json_flag {
        return json::write(&LogoutJson {
            logged_out: true,
            daemon: LogoutDaemonJson {
                running: out.daemon_running,
                in_flight: out.in_flight,
            },
        });
    }
    let mut stdout = std::io::stdout().lock();
    let detail = if out.daemon_running {
        "sync daemon still running (now idle)"
    } else {
        "sync daemon unavailable"
    };
    writeln!(
        stdout,
        "{} logged out (local credentials removed; {detail})",
        "\u{2713}".green()
    )?;
    Ok(())
}
