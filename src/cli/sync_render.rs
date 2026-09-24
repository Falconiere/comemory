//! Rendering for `comemory sync` — the TTY and `--json` shapes of a run,
//! the cursors report, the verify report and the daemon status. Kept apart
//! from `cli/sync.rs`, which owns the arguments and the dispatch, so each
//! side stays under the size ceiling.

use std::io::Write as _;

use crate::cli::output::json;
use crate::domains::sync::code::{self, CodePushStats, NotARepository};
use crate::domains::sync::daemon::{self, DaemonStatus};
use crate::domains::sync::initial::InitialSyncStats;
use crate::domains::sync::manual::RunStats;
use crate::domains::sync::verify;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::{code_sync, indexed_files, repo_marker, sync_log, sync_state};

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
    use std::fmt::Write as _;
    let mut line = format!(
        "code: {} repo(s) pushed · {} files · {} removed · unchanged={} · skip_repos={} · blocked_repo={}",
        code.repos,
        code.files_pushed,
        code.files_removed,
        code.unchanged,
        code.skipped_config,
        code.blocked_repo
    );
    for (label, count) in [
        ("worktrees", code.skipped_worktree),
        ("missing_root", code.skipped_missing_root),
        ("failed", code.failed),
    ] {
        if count > 0 {
            let _ = write!(line, " · {label}={count}");
        }
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
    /// Why this row is never offered, however much it moved: `"worktree"`
    /// (its root is a linked `git worktree`), `"missing_root"` (its root is
    /// not on disk) or `"no_checkout"` (its root is a directory git cannot
    /// open). Absent for a repo the push does offer.
    #[serde(skip_serializing_if = "Option::is_none")]
    withheld: Option<&'static str>,
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
            withheld: code::not_a_repository(conn, &repo)?.map(NotARepository::label),
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
                "code: {} files={} head={} pushed_head={} moved_since_push={}{}",
                row.repo,
                row.files,
                row.head.as_deref().unwrap_or("-"),
                row.pushed_head.as_deref().unwrap_or("never"),
                row.moved_since_push,
                row.withheld
                    .map(|why| format!(" withheld={why}"))
                    .unwrap_or_default()
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

/// A `run` / `push` / `pull` report: one line per leg that ran, or the
/// `--json` object with every leg (`null` for one that did not).
pub(crate) fn emit_run(json_flag: bool, workspace: &str, stats: &RunStats) -> Result<()> {
    if json_flag {
        return json::write(&serde_json::json!({
            "workspace": workspace,
            "push": stats.push,
            "pull": stats.pull,
            "code": stats.code,
            "refresh": stats.refresh,
        }));
    }
    let mut out = std::io::stdout().lock();
    if let Some(p) = &stats.pull {
        writeln!(
            out,
            "Pulled {} entries (seq {})",
            p.pulled, p.last_pulled_seq
        )?;
    }
    if let Some(p) = &stats.push {
        writeln!(
            out,
            "Pushed {} entries (skip_repos={}, blocked_repo={}, blocked_secrets={}, rejected_repo={})",
            p.pushed, p.skipped_config, p.blocked_repo, p.blocked_secrets, p.rejected_repo
        )?;
    }
    if let Some(r) = stats.refresh.as_ref().filter(|r| r.checked > 0) {
        writeln!(
            out,
            "refreshed: {} of {} hooked repo(s)",
            r.refreshed, r.checked
        )?;
        for err in &r.errors {
            writeln!(out, "  refresh failed: {err}")?;
        }
    }
    if let Some(c) = &stats.code {
        writeln!(out, "{}", code_line(c))?;
        for err in &c.errors {
            writeln!(out, "  code push failed: {err}")?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/sync_render.rs"]
mod tests;
