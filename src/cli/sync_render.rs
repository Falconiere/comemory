//! Rendering for `comemory sync` — the TTY and `--json` shapes of a run,
//! the cursors report, the verify report and the daemon status. Kept apart
//! from `cli/sync.rs`, which owns the arguments and the dispatch, so each
//! side stays under the size ceiling.

use std::io::Write as _;

use crate::output::json;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::{code_sync, indexed_files, repo_marker, sync_log, sync_state};
use crate::sync::code::CodePushStats;
use crate::sync::daemon::{self, DaemonStatus};
use crate::sync::initial::InitialSyncStats;
use crate::sync::{pull, push, verify};

/// The login report's code line: what the code-index push did, or why it
/// could not run.
pub(crate) fn code_summary_line(stats: &InitialSyncStats) -> String {
    match &stats.code_error {
        Some(e) => format!("code: not pushed ({e})"),
        None => code_line(&stats.code),
    }
}

/// One line summarizing a code push, shared by login and `comemory sync`.
pub(crate) fn code_line(code: &CodePushStats) -> String {
    let mut line = format!(
        "code: {} repo(s) pushed · {} files · {} removed · unchanged={} · skip_repos={}",
        code.repos, code.files_pushed, code.files_removed, code.unchanged, code.skipped_config
    );
    if code.failed > 0 {
        use std::fmt::Write as _;
        let _ = write!(line, " · failed={}", code.failed);
    }
    line
}

/// One repo's row in `--action status`'s `code` list.
#[derive(serde::Serialize)]
struct CodeStatusRow {
    repo: String,
    files: usize,
    head: Option<String>,
    pushed_head: Option<String>,
    pushed_at: Option<String>,
    /// Whether the local head or mining cursor moved since the last push —
    /// what the next `comemory sync` will offer, judged offline.
    moved_since_push: bool,
}

fn code_status_rows(conn: &Connection) -> Result<Vec<CodeStatusRow>> {
    let mut rows = Vec::new();
    for repo in repo_marker::all_repos(conn)? {
        let head = repo_marker::last_head(conn, &repo)?;
        let mined = repo_marker::last_mined_commit(conn, &repo)?;
        let cursor = code_sync::cursor(conn, &repo)?;
        let moved_since_push = cursor
            .as_ref()
            .is_none_or(|c| c.pushed_head != head || c.pushed_mined_commit != mined);
        rows.push(CodeStatusRow {
            files: indexed_files::list_for_repo(conn, &repo)?.len(),
            head,
            pushed_head: cursor.as_ref().and_then(|c| c.pushed_head.clone()),
            pushed_at: cursor.map(|c| c.pushed_at),
            moved_since_push,
            repo,
        });
    }
    Ok(rows)
}

pub(crate) fn emit_daemon_status(json_flag: bool, st: &DaemonStatus) -> Result<()> {
    if json_flag {
        json::write(st)?;
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(out, "platform: {}", st.platform)?;
        if let Some(path) = &st.unit_path {
            writeln!(out, "unit: {path}")?;
        }
        writeln!(out, "installed: {}", st.installed)?;
        writeln!(out, "running: {}", st.running)?;
        writeln!(out, "detail: {}", st.detail)?;
        if let Some(warn) = st.inactive_warning() {
            writeln!(out, "warning: {warn}")?;
        }
    }
    Ok(())
}

pub(crate) fn emit_status(json_flag: bool, conn: &mut Connection, workspace: &str) -> Result<()> {
    let row = sync_state::get(conn, workspace)?;
    let head = sync_log::head_seq(conn)?;
    let (pushed, pulled, last_sync) = row.as_ref().map_or((0, 0, None), |r| {
        (r.pushed_seq, r.pulled_seq, r.last_sync_at.clone())
    });
    let pending = sync_log::pending_local(conn, pushed)?;
    let code = code_status_rows(conn)?;
    let daemon = match daemon::status() {
        Ok(st) => st,
        Err(_) => DaemonStatus {
            platform: "unknown",
            unit_path: None,
            installed: false,
            running: false,
            detail: "daemon status unavailable".into(),
        },
    };
    if json_flag {
        json::write(&serde_json::json!({
            "workspace": workspace,
            "pushed_seq": pushed,
            "pulled_seq": pulled,
            "head_seq": head,
            "pending": pending,
            "last_sync_at": last_sync,
            "daemon": daemon,
            "code": code,
        }))?;
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(out, "workspace: {workspace}")?;
        writeln!(out, "pushed_seq: {pushed}")?;
        writeln!(out, "pulled_seq: {pulled}")?;
        writeln!(out, "head_seq: {head}")?;
        writeln!(out, "pending: {pending}")?;
        writeln!(
            out,
            "daemon: installed={} running={} ({})",
            daemon.installed, daemon.running, daemon.detail
        )?;
        for row in &code {
            writeln!(
                out,
                "code: {} files={} head={} pushed_head={} moved_since_push={}",
                row.repo,
                row.files,
                row.head.as_deref().unwrap_or("-"),
                row.pushed_head.as_deref().unwrap_or("never"),
                row.moved_since_push
            )?;
        }
        if let Some(warn) = daemon.inactive_warning() {
            writeln!(out, "warning: {warn}")?;
        }
    }
    Ok(())
}

pub(crate) fn emit_verify(json_flag: bool, report: &verify::VerifyReport) -> Result<()> {
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

pub(crate) fn emit_run(
    json_flag: bool,
    workspace: &str,
    pull_stats: Option<&pull::PullStats>,
    push_stats: Option<&push::PushStats>,
    code_stats: Option<&CodePushStats>,
) -> Result<()> {
    if json_flag {
        json::write(&serde_json::json!({
            "workspace": workspace,
            "push": push_stats,
            "pull": pull_stats,
            "code": code_stats,
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
                "Pushed {} entries (skip_repos={}, blocked_secrets={}, rejected_repo={})",
                p.pushed, p.skipped_config, p.blocked_secrets, p.rejected_repo
            )?;
        }
        if let Some(c) = code_stats {
            writeln!(out, "{}", code_line(c))?;
            for err in &c.errors {
                writeln!(out, "  code push failed: {err}")?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/sync_render.rs"]
mod tests;
