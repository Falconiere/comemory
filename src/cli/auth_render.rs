//! JSON shapes and TTY rendering for `comemory auth`.

use std::io::Write as _;

use owo_colors::OwoColorize;
use serde::Serialize;

use crate::cli::output::json;
use crate::domains::sync::auth_file::AuthFile;
use crate::domains::sync::cloud::StatusReport;
use crate::domains::sync::daemon;
use crate::prelude::*;

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

#[derive(Serialize)]
pub(crate) struct DaemonLoginJson {
    pub skipped: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub running: Option<bool>,
}

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
    pub code: Option<crate::domains::sync::code::CodePushStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct LogoutJson {
    pub logged_out: bool,
    pub daemon_stopped: bool,
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
            code: Some(s.code.clone()),
            code_error: s.code_error.clone(),
            error: None,
        },
        Err(e) => InitialSyncJson {
            ok: false,
            pulled: None,
            pushed: None,
            skipped_config: None,
            code: None,
            code_error: None,
            error: Some(e.to_string()),
        },
    }
}

pub(crate) fn emit_logged_out(json_flag: bool) -> Result<()> {
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

pub(crate) fn emit_status(json_flag: bool, report: &StatusReport) -> Result<()> {
    let daemon_st = if report.authenticated {
        daemon::status().ok()
    } else {
        None
    };
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

pub(crate) fn write_logout(json_flag: bool, daemon_stopped: bool) -> Result<()> {
    if json_flag {
        return json::write(&LogoutJson {
            logged_out: true,
            daemon_stopped,
        });
    }
    let mut out = std::io::stdout().lock();
    if daemon_stopped {
        writeln!(
            out,
            "{} logged out (local credentials removed; sync daemon stopped)",
            "\u{2713}".green()
        )?;
    } else {
        writeln!(
            out,
            "{} logged out (local credentials removed; sync daemon still running — run `comemory sync daemon stop`)",
            "\u{2713}".green()
        )?;
    }
    Ok(())
}
