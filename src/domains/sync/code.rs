//! Push the code index to the organization's workspace, the way memories
//! already sync — so the console's Code graph fills in right after
//! `comemory auth login`, with no further step.
//!
//! What leaves the machine is the snippet-free projection
//! `domains::sync::exchange::code_types` describes: file paths and blob OIDs, symbol
//! names with kinds and line ranges, the resolved `imports` edges and the
//! mined `co_changed` pairs. Never a line of source — see
//! [`project_file`](crate::domains::sync::code::project_file), the one place a file entry is built.
//!
//! Every indexed repo (`repo_marker`) is offered unless its label matches
//! `[sync] skip_repos` or `[sync] code_index` is off. The unit of work is
//! one repo: read the workspace's manifest, diff it against
//! `indexed_files` by blob OID ([`crate::domains::sync::code_plan`]), and post only
//! what differs. A repo that fails leaves the others alone — its error is
//! counted and reported, and the next run re-offers it.
//!
//! Two entry points differ only in when they skip the manifest read:
//! [`run_code_push`](crate::domains::sync::code::run_code_push) (a manual `comemory sync`, login) always asks the
//! workspace; [`run_code_push_if_moved`](crate::domains::sync::code::run_code_push_if_moved) (the daemon, the tail of
//! `index-code`) first compares the recorded cursor against the local
//! head, mining cursor and file digest, and stays silent when nothing moved.

use std::collections::BTreeSet;

use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;

use crate::config::{Config, Paths};
use crate::domains::sync::AuthFile;
use crate::domains::sync::exchange::{CoChangeWire, CodeFileWire, CodeSymbolWire};
use crate::domains::sync::{client_code, code_plan};
use crate::prelude::*;
use crate::store::code_sync::{self, CodeSyncCursor};
use crate::store::{Connection, connection, indexed_files, repo_marker};

/// Counters surfaced by `comemory sync` and the login report.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct CodePushStats {
    /// Repos that sent at least one batch.
    pub repos: u32,
    /// Files the workspace applied across every batch.
    pub files_pushed: u32,
    /// Paths the workspace removed.
    pub files_removed: u32,
    /// Import batches posted.
    pub batches: u32,
    /// Repos whose workspace copy already matched.
    pub unchanged: u32,
    /// Repos withheld by `[sync] skip_repos`.
    pub skipped_config: u32,
    /// Repos whose push failed; `errors` names each.
    pub failed: u32,
    /// One line per failed repo.
    pub errors: Vec<String>,
}

/// Push every indexed repo, reading each repo's manifest first.
///
/// # Errors
/// Config and store failures. A repo's transport failure is counted in
/// `failed`, not returned — the other repos still push.
pub fn run_code_push(
    cfg: &Config,
    conn: &mut Connection,
    auth: &AuthFile,
) -> Result<CodePushStats> {
    run(cfg, conn, auth, true, None)
}

/// Push only the repos whose local index moved since their last push.
///
/// # Errors
/// As [`run_code_push`].
pub fn run_code_push_if_moved(
    cfg: &Config,
    conn: &mut Connection,
    auth: &AuthFile,
) -> Result<CodePushStats> {
    run(cfg, conn, auth, false, None)
}

/// The tail of a CLI `index-code`: push that one repo if it moved. Never
/// fails the index run — being offline, or logged out, is ordinary.
pub fn after_index_best_effort(paths: &Paths, cfg: &Config, repo: &str) {
    let outcome = (|| -> Result<Option<CodePushStats>> {
        let Some(auth) = AuthFile::load_usable(paths)? else {
            return Ok(None);
        };
        let mut conn = connection::open(paths.db_path())?;
        run(cfg, &mut conn, &auth, false, Some(repo)).map(Some)
    })();
    match outcome {
        Ok(Some(stats)) => tracing::debug!(
            files = stats.files_pushed,
            batches = stats.batches,
            failed = stats.failed,
            "code index push after index-code"
        ),
        Ok(None) => tracing::debug!("code index push skipped: not logged in"),
        Err(e) => tracing::debug!(error = %e, "code index push after index-code failed"),
    }
}

fn run(
    cfg: &Config,
    conn: &mut Connection,
    auth: &AuthFile,
    force: bool,
    only: Option<&str>,
) -> Result<CodePushStats> {
    let mut stats = CodePushStats::default();
    if !cfg.sync.code_index {
        return Ok(stats);
    }
    let skip = cfg.sync.skip_matcher()?;
    for repo in repo_marker::all_repos(conn)? {
        if only.is_some_and(|wanted| wanted != repo) {
            continue;
        }
        if skip.is_skipped(&repo) {
            stats.skipped_config += 1;
            continue;
        }
        if let Err(e) = push_repo(conn, auth, &repo, force, &mut stats) {
            tracing::warn!(repo = %repo, error = %e, "code index push failed");
            stats.failed += 1;
            stats.errors.push(format!("{repo}: {e}"));
        }
    }
    Ok(stats)
}

/// The local state a push is keyed on: head, mining cursor, and a digest
/// over every `(path, blob_oid)` row — so a staged edit that moved a blob
/// without moving HEAD still counts as movement.
struct LocalState {
    head: Option<String>,
    mined: Option<String>,
    files: Vec<(String, String)>,
    digest: String,
}

fn local_state(conn: &Connection, repo: &str) -> Result<LocalState> {
    let files = indexed_files::list_for_repo(conn, repo)?;
    let mut hasher = Sha256::new();
    for (path, oid) in &files {
        hasher.update(path.as_bytes());
        hasher.update(b"\0");
        hasher.update(oid.as_bytes());
        hasher.update(b"\n");
    }
    let digest = hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
            out
        });
    Ok(LocalState {
        head: repo_marker::last_head(conn, repo)?,
        mined: repo_marker::last_mined_commit(conn, repo)?,
        files,
        digest,
    })
}

fn push_repo(
    conn: &Connection,
    auth: &AuthFile,
    repo: &str,
    force: bool,
    stats: &mut CodePushStats,
) -> Result<()> {
    let local = local_state(conn, repo)?;
    if !force && code_sync::cursor(conn, repo)?.is_some_and(|c| c.matches(&local)) {
        stats.unchanged += 1;
        return Ok(());
    }
    let secret = auth.effective_secret();
    let manifest = client_code::fetch_code_manifest(&auth.api_url, &secret, repo)?;
    let plan = code_plan::plan(
        &local.files,
        &manifest.files,
        local.head.as_deref(),
        manifest.head.as_deref(),
        local.mined.as_deref(),
        manifest.mined_commit.as_deref(),
    );
    if plan.is_empty() {
        record_cursor(conn, repo, &local)?;
        stats.unchanged += 1;
        return Ok(());
    }
    let known: BTreeSet<&str> = local.files.iter().map(|(path, _)| path.as_str()).collect();
    let files = plan
        .changed
        .iter()
        .map(|path| project_file(conn, repo, path, &known))
        .collect::<Result<Vec<_>>>()?;
    let cochange = if plan.send_cochange {
        Some(
            code_sync::co_changed_pairs(conn, repo)?
                .into_iter()
                .map(|(from, to, weight)| CoChangeWire { from, to, weight })
                .collect(),
        )
    } else {
        None
    };
    let requests = code_plan::batches(
        repo,
        local.head.as_deref(),
        local.mined.as_deref(),
        files,
        plan.removed,
        cochange,
    )?;
    for req in &requests {
        let resp = client_code::push_code_import(&auth.api_url, &secret, req)?;
        if let Some(first) = resp.rejected.first() {
            return Err(Error::Other(format!(
                "workspace rejected the code import ({} entries; first: {} {})",
                resp.rejected.len(),
                first.path,
                first.reason
            )));
        }
        stats.files_pushed = stats.files_pushed.saturating_add(count(resp.applied));
        stats.files_removed = stats.files_removed.saturating_add(count(resp.removed));
        stats.batches += 1;
    }
    stats.repos += 1;
    record_cursor(conn, repo, &local)
}

/// One file's projection: its blob, its top-level symbols, and the imports
/// that resolve onto a path this index knows — a stale edge onto a deleted
/// file is not worth a ghost node on the workspace.
///
/// # Errors
/// Store failures; a path with no `indexed_files` row is [`Error::NotFound`].
pub fn project_file(
    conn: &Connection,
    repo: &str,
    path: &str,
    known: &BTreeSet<&str>,
) -> Result<CodeFileWire> {
    let blob_oid = indexed_files::blob_oid_for(conn, repo, path)?
        .ok_or_else(|| Error::NotFound(format!("indexed file {repo}:{path}")))?;
    let symbols = code_sync::parent_symbols_for_file(conn, repo, path)?
        .into_iter()
        .map(|s| CodeSymbolWire {
            symbol: s.symbol,
            kind: s.kind,
            lang: s.lang,
            line_start: s.line_start,
            line_end: s.line_end,
        })
        .collect();
    let imports = code_sync::import_targets(conn, repo, path)?
        .into_iter()
        .filter(|target| known.contains(target.as_str()))
        .collect();
    Ok(CodeFileWire {
        path: path.to_owned(),
        blob_oid,
        symbols,
        imports,
    })
}

impl CodeSyncCursor {
    fn matches(&self, local: &LocalState) -> bool {
        self.pushed_head == local.head
            && self.pushed_mined_commit == local.mined
            && self.pushed_digest == local.digest
    }
}

fn record_cursor(conn: &Connection, repo: &str, local: &LocalState) -> Result<()> {
    let pushed_at = OffsetDateTime::now_utc()
        .format(&Iso8601::DEFAULT)
        .map_err(|e| Error::Other(format!("timestamp: {e}")))?;
    code_sync::set_cursor(
        conn,
        repo,
        &CodeSyncCursor {
            pushed_head: local.head.clone(),
            pushed_mined_commit: local.mined.clone(),
            pushed_digest: local.digest.clone(),
            pushed_at,
        },
    )
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
#[path = "tests/code.rs"]
mod tests;
