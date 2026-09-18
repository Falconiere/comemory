//! Lazy auto-reindex: the non-blocking background trigger shared by
//! `search-code` and `context`. Under `AutoReindexMode::Lazy`, when the
//! command runs inside a git repo with a stale code index, spawn a DETACHED
//! `index-code` and return at once. Best-effort throughout: failures are
//! logged and swallowed so the search never blocks or fails.
//!
//! This file owns only the parts that touch the process: the orchestration in
//! [`maybe_trigger`] and the detached launch in `spawn_index_code`. *Whether* a
//! reindex is due — the staleness probe, the `schema_meta` debounce marker, and
//! the repo label/root resolution — is code-capability policy and lives in
//! [`crate::domains::code::reindex_policy`] (#167).

use std::path::Path;
use std::process::Command;

use crate::config::paths::Paths;
use crate::config::{AutoReindexMode, Config};
use crate::domains::code::reindex_policy::{
    RepoContext, now_millis, read_last_trigger, read_repo_marker, record_trigger, repo_context,
    same_root, should_reindex,
};
use crate::store::Connection;

/// Best-effort lazy reindex entry point for `search-code` / `context`.
///
/// Resolves the repo from the CWD, runs the cheap staleness probe (current
/// HEAD via one `git2` resolve vs `repo_marker.last_mined_commit` — no
/// working-tree walk, so uncommitted edits are intentionally not detected),
/// consults [`should_reindex`], records the `schema_meta` debounce marker
/// (`lazy_reindex_head:<repo>` = `"<head>|<unix_millis>"`), then spawns a
/// detached `index-code`. Every failure (no repo, unborn HEAD, marker or
/// spawn error) is logged and swallowed — the search proceeds against the
/// current (possibly slightly stale) index regardless.
pub(crate) fn maybe_trigger(
    conn: &Connection,
    cfg: &Config,
    paths: &Paths,
    repo_filter: Option<&str>,
) {
    if !matches!(cfg.indexing.auto_reindex, AutoReindexMode::Lazy) {
        return;
    }
    let Some(ctx) = repo_context(repo_filter) else {
        // Off-repo (no git repo at CWD) or bare repo: nothing to reindex.
        return;
    };
    let current_head = match crate::domains::code::git_utils::current_head(&ctx.root) {
        Ok(h) => h,
        Err(e) => {
            tracing::debug!(error = %e, "lazy reindex: HEAD unresolved; skipping");
            return;
        }
    };
    let marker = read_repo_marker(conn, &ctx.repo);
    // Archived (`repo_marker.archived`, the console's archive action): the
    // repo stays searchable but is deliberately no longer indexed, so a
    // stale HEAD must not fire a background reindex behind the user's back.
    if marker.as_ref().is_some_and(|m| m.archived) {
        tracing::debug!(repo = %ctx.repo, "lazy reindex: repo is archived; skipping");
        return;
    }
    // Label/checkout collision guard: when the repo was already indexed from
    // a DIFFERENT working-tree root, the CWD is not that checkout (it just
    // reuses the label), so reindexing it would corrupt the foreign repo's
    // rows. Skip. A NULL/absent root (never indexed, or pre-v7) is allowed —
    // the never-indexed case must still be able to fire.
    if let Some(root) = marker.as_ref().and_then(|m| m.root_path.as_deref())
        && !same_root(root, &ctx.root)
    {
        tracing::debug!(
            repo = %ctx.repo,
            indexed_root = %root,
            cwd_root = %ctx.root.display(),
            "lazy reindex: CWD is not the indexed checkout for this label; skipping",
        );
        return;
    }
    let last_indexed = marker.and_then(|m| m.last_mined_commit);
    let last_trigger = read_last_trigger(conn, &ctx.repo);
    let now = now_millis();
    if !should_reindex(
        &cfg.indexing.auto_reindex,
        &current_head,
        last_indexed.as_deref(),
        last_trigger.as_ref(),
        now,
        cfg.indexing.auto_reindex_threshold_ms,
    ) {
        return;
    }
    // Record the trigger BEFORE spawning so a concurrent search in the
    // debounce window sees the marker even if the spawn is slow.
    record_trigger(conn, &ctx.repo, &current_head, now);
    spawn_index_code(&ctx, paths.data_dir());
}

/// Spawn a DETACHED `comemory index-code --repo <repo> --path <root>
/// --data-dir <dir>` and return immediately without awaiting it.
///
/// The child's stdio is redirected to null so it never writes to the
/// caller's terminal, and the [`std::process::Child`] handle is dropped
/// (not waited on) so the search returns without blocking on the index.
/// `--data-dir` pins the spawned reindex to the SAME store the search is
/// reading. Best-effort: a missing `current_exe` or a spawn failure is
/// logged via `tracing` and swallowed — a reindex that cannot start must
/// never surface as a search error.
fn spawn_index_code(ctx: &RepoContext, data_dir: &Path) {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!(error = %e, "lazy reindex: current_exe unavailable; skipping spawn");
            return;
        }
    };
    let mut cmd = Command::new(exe);
    cmd.arg("index-code")
        .arg("--repo")
        .arg(&ctx.repo)
        .arg("--path")
        .arg(&ctx.root)
        .arg("--data-dir")
        .arg(data_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    match cmd.spawn() {
        Ok(child) => {
            tracing::debug!(
                repo = %ctx.repo,
                pid = child.id(),
                "lazy reindex: detached index-code spawned",
            );
            // Drop the handle without waiting: the child runs to completion
            // independently and the OS reaps it (on unix it is reparented to
            // init once this process exits).
            drop(child);
        }
        Err(e) => {
            tracing::debug!(error = %e, repo = %ctx.repo, "lazy reindex: index-code spawn failed");
        }
    }
}
