//! `comemory sync` — push/pull against the platform. CLI-only.

use std::io::Write as _;
use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};

use crate::cli::load_config;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::config::sync::apply_embed_model;
use crate::output::json;
use crate::prelude::*;
use crate::store::connection::open;
use crate::store::sync_state;
use crate::sync::auth_file::AuthFile;
use crate::sync::{pull, push, verify};

const EXAMPLES: &str = "\
Examples:
  comemory sync
  comemory sync --action push
  comemory sync --action pull --workspace ws_abc
  comemory sync --action status --json
  comemory sync --action verify
  comemory sync --allow-secret deadbeef";

/// Sync mode — replaces four separate bool flags (clippy `struct_excessive_bools`).
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum SyncAction {
    /// Push then pull (default).
    #[default]
    Run,
    /// Push local changes only (`--push-only` alias).
    #[value(name = "push", alias = "push-only")]
    Push,
    /// Pull remote changes only (`--pull-only` alias).
    #[value(name = "pull", alias = "pull-only")]
    Pull,
    /// Compare local/remote manifests and repair differing buckets (AC-9).
    Verify,
    /// Print sync cursors.
    Status,
}

/// Arguments to `comemory sync`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Operation: `run` (default), `push`, `pull`, `verify`, or `status`.
    #[arg(long, value_enum, default_value_t = SyncAction::Run)]
    pub action: SyncAction,
    /// Platform workspace id (falls back to config default or personal workspace).
    #[arg(long)]
    pub workspace: Option<String>,
    /// Record a secret-scan override for one memory id before push.
    #[arg(long, value_name = "ID")]
    pub allow_secret: Option<String>,
}

/// Run push/pull/status/verify for the resolved workspace.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let cfg = load_config(&paths)?;
    let auth = AuthFile::load(&paths)?
        .ok_or_else(|| Error::Usage("not logged in — run `comemory auth login`".into()))?;
    let workspace = resolve_workspace(a.workspace.as_deref(), &cfg, &auth);
    let mut conn = open(paths.db_path())?;
    apply_embed_model(&conn, &cfg.embed)?;

    match a.action {
        SyncAction::Status => emit_status(json_flag, &mut conn, &workspace),
        SyncAction::Verify => {
            let report = verify::verify_manifests(&paths, &cfg, &mut conn, &auth, &workspace)?;
            emit_verify(json_flag, &report)
        }
        SyncAction::Push => {
            let stats = push::run_push(
                &paths,
                &cfg,
                &mut conn,
                &auth,
                &workspace,
                a.allow_secret.as_deref(),
                2000,
            )?;
            emit_run(json_flag, &workspace, None, Some(&stats))
        }
        SyncAction::Pull => {
            let stats = pull::run_pull(&paths, &cfg, &mut conn, &auth, &workspace, 2000)?;
            emit_run(json_flag, &workspace, Some(&stats), None)
        }
        SyncAction::Run => {
            let pull_stats = pull::run_pull(&paths, &cfg, &mut conn, &auth, &workspace, 2000)?;
            let push_stats = push::run_push(
                &paths,
                &cfg,
                &mut conn,
                &auth,
                &workspace,
                a.allow_secret.as_deref(),
                2000,
            )?;
            emit_run(json_flag, &workspace, Some(&pull_stats), Some(&push_stats))
        }
    }
}

fn resolve_workspace(flag: Option<&str>, cfg: &crate::config::Config, auth: &AuthFile) -> String {
    flag.map(str::to_owned)
        .or_else(|| cfg.sync.default_workspace.clone())
        .unwrap_or_else(|| auth.personal_workspace_id.clone())
}

fn emit_status(json_flag: bool, conn: &mut rusqlite::Connection, workspace: &str) -> Result<()> {
    let row = sync_state::get(conn, workspace)?;
    let head = crate::store::sync_log::head_seq(conn)?;
    let (pushed, pulled, last_sync) = row.as_ref().map_or((0, 0, None), |r| {
        (r.pushed_seq, r.pulled_seq, r.last_sync_at.clone())
    });
    if json_flag {
        json::write(&serde_json::json!({
            "workspace": workspace,
            "pushed_seq": pushed,
            "pulled_seq": pulled,
            "head_seq": head,
            "last_sync_at": last_sync,
        }))?;
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(out, "workspace: {workspace}")?;
        writeln!(out, "pushed_seq: {pushed}")?;
        writeln!(out, "pulled_seq: {pulled}")?;
        writeln!(out, "head_seq: {head}")?;
    }
    Ok(())
}

fn emit_verify(json_flag: bool, report: &verify::VerifyReport) -> Result<()> {
    if json_flag {
        json::write(report)?;
    } else {
        let mut out = std::io::stdout().lock();
        if report.differing_buckets == 0 {
            let suffix = if report.repaired { " after repair" } else { "" };
            writeln!(
                out,
                "Manifests match{suffix} (head local={}, remote={})",
                report.local_head_seq, report.remote_head_seq
            )?;
        } else {
            writeln!(
                out,
                "{} bucket(s) still differ after repair (local head={}, remote head={})",
                report.differing_buckets, report.local_head_seq, report.remote_head_seq
            )?;
        }
    }
    Ok(())
}

fn emit_run(
    json_flag: bool,
    workspace: &str,
    pull_stats: Option<&pull::PullStats>,
    push_stats: Option<&push::PushStats>,
) -> Result<()> {
    if json_flag {
        json::write(&serde_json::json!({
            "workspace": workspace,
            "push": push_stats,
            "pull": pull_stats,
        }))?;
    } else {
        let mut out = std::io::stdout().lock();
        if let Some(p) = pull_stats {
            writeln!(
                out,
                "Pulled {} entries (seq {})",
                p.pulled, p.last_pulled_seq
            )?;
        }
        if let Some(p) = push_stats {
            writeln!(
                out,
                "Pushed {} entries (skipped personal={}, not_in_org={}, ambiguous={}, blocked_secrets={})",
                p.pushed,
                p.skipped_personal,
                p.skipped_not_in_org,
                p.skipped_ambiguous,
                p.blocked_secrets
            )?;
        }
    }
    Ok(())
}
