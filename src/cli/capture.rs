//! `comemory capture` — post a redacted session receipt (CLI-only).

use std::io::{Read as _, Write as _};
use std::path::PathBuf;

use clap::{Args as ClapArgs, Subcommand};

use crate::cli::off_runtime::off_runtime;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::domains::capture;
use crate::domains::sync::AuthFile;
use crate::output::json;
use crate::prelude::*;

const EXAMPLES: &str = "\
Examples:
  comemory capture session --path ~/.claude/projects/.../session.jsonl --dry-run
  comemory capture session --session-id 8e9f54e3-a984-46ab-8403-135ee920cbca
  comemory capture sources
  comemory capture install-hook";

/// Top-level `capture` args — nested subcommand required.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Nested action.
    #[command(subcommand)]
    pub cmd: CaptureCmd,
}

/// Nested capture subcommands.
#[derive(Subcommand, Debug)]
pub enum CaptureCmd {
    /// Build (and optionally POST) one session receipt.
    Session(SessionArgs),
    /// Show platform capture-consent rows (CLI cannot grant consent).
    Sources,
    /// Install a Claude Code SessionEnd hook that runs `comemory capture`.
    InstallHook(InstallHookArgs),
}

/// Args for `capture session`.
#[derive(ClapArgs, Debug)]
pub struct SessionArgs {
    /// Capture source (default `claude-code`).
    #[arg(long, default_value = "claude-code")]
    pub source: String,
    /// Explicit transcript JSONL path.
    #[arg(long)]
    pub path: Option<PathBuf>,
    /// Tool session id to locate under `~/.claude/projects`.
    #[arg(long)]
    pub session_id: Option<String>,
    /// Read Claude Code SessionEnd JSON from stdin for session id / path.
    #[arg(long, default_value_t = false)]
    pub from_hook: bool,
    /// Build the receipt without POSTing.
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
    /// Post even when local redaction found secrets (still redacts + attests).
    #[arg(long, default_value_t = false)]
    pub allow_secret: bool,
}

/// Args for `capture install-hook`.
#[derive(ClapArgs, Debug)]
pub struct InstallHookArgs {
    /// Settings file to write (default: `.claude/settings.json` under cwd).
    #[arg(long)]
    pub settings: Option<PathBuf>,
    /// Replace an existing non-comemory SessionEnd block.
    #[arg(long, default_value_t = false)]
    pub force: bool,
}

/// Dispatch capture / sources / install-hook.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    match a.cmd {
        CaptureCmd::Sources => run_sources(json_flag, data_dir).await,
        CaptureCmd::InstallHook(h) => run_install_hook(h, json_flag),
        CaptureCmd::Session(s) => run_session(s, json_flag, data_dir).await,
    }
}

async fn run_sources(json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let auth = AuthFile::load(&paths)?
        .ok_or_else(|| Error::Usage("not logged in — run `comemory auth login`".into()))?;
    let rows = off_runtime(move || capture::run_sources(&auth))?;
    if json_flag {
        json::write(&serde_json::json!({ "sources": rows }))?;
    } else {
        let mut out = std::io::stdout().lock();
        if rows.is_empty() {
            writeln!(out, "no capture sources returned")?;
        } else {
            for r in &rows {
                let updated = r.updated_at.as_deref().unwrap_or("-");
                writeln!(
                    out,
                    "{}  enabled={}  updatedAt={}",
                    r.source, r.enabled, updated
                )?;
            }
            writeln!(
                out,
                "Consent is granted in the console (or with a workspace API key); this CLI cannot enable capture."
            )?;
        }
    }
    Ok(())
}

fn run_install_hook(h: InstallHookArgs, json_flag: bool) -> Result<()> {
    let path = h
        .settings
        .unwrap_or_else(|| PathBuf::from(capture::hook::DEFAULT_SETTINGS_PATH));
    let report = capture::hook::install(&path, h.force)?;
    if json_flag {
        json::write(&report)?;
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(
            out,
            "Installed SessionEnd hook in {}:\n  {}",
            report.path.display(),
            report.command
        )?;
    }
    Ok(())
}

async fn run_session(a: SessionArgs, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let auth = AuthFile::load(&paths)?
        .ok_or_else(|| Error::Usage("not logged in — run `comemory auth login`".into()))?;
    let (path, session_id) = resolve_target(&a)?;
    let req = capture::CaptureRequest {
        source: a.source,
        path,
        session_id,
        dry_run: a.dry_run,
        allow_secret: a.allow_secret,
    };
    let report = off_runtime(move || capture::run_capture(&auth, req))?;
    emit_report(json_flag, &report)
}

/// Read stdin when `--from-hook`, else take the flags clap already parsed.
///
/// Decoding the payload belongs to [`capture::hook`], which also owns the
/// command that produces it; this side only acquires the bytes.
fn resolve_target(a: &SessionArgs) -> Result<(Option<PathBuf>, Option<String>)> {
    if a.from_hook {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        return capture::hook::session_end_target(&buf);
    }
    Ok((a.path.clone(), a.session_id.clone()))
}

fn emit_report(json_flag: bool, report: &capture::CaptureReport) -> Result<()> {
    if json_flag {
        json::write(report)?;
        return Ok(());
    }
    let mut out = std::io::stdout().lock();
    let r = &report.receipt;
    writeln!(
        out,
        "source={} externalId={} turns={} bytes={} digest={}",
        r.source, r.external_id, r.turn_count, r.transcript_bytes, r.transcript_digest
    )?;
    if r.redaction.findings.is_empty() {
        writeln!(out, "redaction v{}: (no findings)", r.redaction.version)?;
    } else {
        write!(out, "redaction v{}:", r.redaction.version)?;
        for f in &r.redaction.findings {
            write!(out, " {}×{}", f.rule, f.count)?;
        }
        writeln!(out)?;
    }
    if report.dry_run {
        writeln!(out, "dry-run: not posted")?;
    } else if let Some(resp) = &report.response {
        writeln!(
            out,
            "posted: created={} session.id={}",
            resp.created,
            resp.session
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
        )?;
    }
    Ok(())
}
