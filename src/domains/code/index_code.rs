//! `index_code::{Request, Response, run}` — shared middle of
//! `comemory index-code`'s DB-write path / `POST /api/v1/code/index`: walk
//! a real git repo and mirror its symbols into `code_symbols` (+
//! `code_fts`), then best-effort refresh the code graph. Moved out of
//! `cli::index_code::run` (Binding Rule 1).
//!
//! `--extract` stays CLI-only (documented HTTP-mapping exclusion): it
//! streams JSONL to stdout and never touches the DB, so `Request` carries
//! no `extract` field and this module only ever runs the DB-write branch.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use git2::Repository;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::domains::code::git_utils::{self, map_git_err};
use crate::prelude::*;
use crate::store::{Connection, code_row, connection::write_transaction, memory_row, repo_marker};
use crate::utilities::activity::{self, Outcome, command};
use crate::utilities::context::Ctx;
use crate::utilities::progress::ProgressSink;

mod embedding;
mod runner;
/// File-walk / symbol-write internals, shared (in part) with
/// `cli::index_code`'s `--extract` path.
pub mod walk;

/// How much of the repo an `index-code` run re-extracts.
#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum IndexMode {
    /// Only files whose working bytes changed since the last run.
    #[default]
    Incremental,
    /// Every file is re-extracted. Vectors for unchanged symbols survive;
    /// per-symbol access counters reset and `code_feedback` survives.
    Full,
}

impl IndexMode {
    /// The `index_runs.mode` slug.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Incremental => "incremental",
            Self::Full => "full",
        }
    }
}

/// `comemory index-code` (DB-write path) / `POST /api/v1/code/index`
/// request. No `extract` field — see the module doc.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Repo label stored alongside each symbol row.
    pub repo: String,
    /// Root to walk. The legacy entry point requires a Git repository.
    pub path: String,
    /// `incremental` (default) or `full` — see [`IndexMode`].
    #[serde(default)]
    pub mode: IndexMode,
}

/// `POST /api/v1/code/index` response. `cli::index_code::run` emits
/// nothing on success (silent, unchanged); the HTTP job form carries a
/// small real count instead of an empty object.
#[derive(Serialize, Debug)]
pub struct Response {
    /// The `req.repo` label this run indexed.
    pub repo: String,
    /// Files actually (re)indexed — those skipped by the fingerprint gate
    /// (no language, untracked, unchanged since the last run) don't count.
    pub files_indexed: usize,
    /// The mode this run used.
    pub mode: IndexMode,
    /// Parent symbols embedded during this run.
    pub embedded: usize,
    /// Embeddings that failed or became stale during inference.
    pub embedding_failed: usize,
    /// Parent symbols still without a vector after this run.
    pub embedding_pending: usize,
    /// Files whose content and extraction were already current.
    pub unchanged: usize,
    /// Previously indexed files removed from this source.
    pub removed: usize,
    /// Supported-code paths rejected as binary or oversized.
    pub unsupported: usize,
}

/// `comemory index-code` (DB-write path) / `POST /api/v1/code/index`: walk
/// `req.path` and mirror its symbols into `code_symbols` (+ `code_fts`)
/// with no progress reporting. See [`run_with_progress`] for the
/// progress-reporting form the job worker uses.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Response> {
    run_with_progress(ctx, req, None)
}

/// Reconcile supported code in a folder, including files outside Git, with
/// optional cancellation and progress. The caller supplies a root-specific repo identity.
pub fn run_folder_with_progress(
    ctx: &mut Ctx<'_>,
    req: Request,
    sink: Option<&dyn ProgressSink>,
) -> Result<Response> {
    index(ctx, req, sink, true)
}

/// Same as [`run`], but reports progress and log lines through `sink` (see
/// [`ProgressSink`]) when one is given.
///
/// The whole walk runs in one SQLite transaction, so a mid-walk failure
/// (or a cooperative cancel) rolls back cleanly: no partial
/// `code_symbols`/`code_fts` rows, no stale `indexed_files` cursors, no
/// duplicate symbol rows on a re-run. Every run — `ok`, `error`, or
/// `cancelled` — is recorded in `index_runs` afterwards ([`runner::record_run`]),
/// so the console's run history and "last run" tile have a source; the
/// recording is best-effort and never turns an indexed repo into a failure.
pub fn run_with_progress(
    ctx: &mut Ctx<'_>,
    req: Request,
    sink: Option<&dyn ProgressSink>,
) -> Result<Response> {
    let activity_started = Instant::now();
    let repo = req.repo.clone();
    let result = index(ctx, req, sink, false);
    let summary = result.as_ref().map(|r| {
        serde_json::json!({
            "repo": r.repo,
            "files": r.files_indexed,
            "mode": r.mode.as_str(),
        })
    });
    let outcome = match &summary {
        Ok(value) => Outcome::Ok(value),
        Err(e) => Outcome::Failed(e),
    };
    let repo = (!repo.is_empty()).then_some(repo.as_str());
    activity::record_in(ctx, command::INDEX_CODE, activity_started, &outcome, repo);
    result
}

/// The walk itself, wrapped by [`run_with_progress`] so the activity row is
/// written once whichever entry point was used — the CLI's [`run`] and the
/// server's job both land here.
fn index(
    ctx: &mut Ctx<'_>,
    mut req: Request,
    sink: Option<&dyn ProgressSink>,
    folder: bool,
) -> Result<Response> {
    let started = Instant::now();
    let started_at = memory_row::iso_format(OffsetDateTime::now_utc())?;
    let root = PathBuf::from(&req.path);
    let excluded = ctx.paths.data_dir().canonicalize().ok();
    let max_bytes = ctx.cfg.indexing.max_file_bytes;
    let embed_cmd = ctx.cfg.code_embed_cmd.clone();
    let embed_model = embedding::provider_id(
        embed_cmd.as_deref().unwrap_or_default(),
        ctx.cfg.code_embed_hint.as_deref(),
    );
    // Reject non-Git legacy sources before opening the database.
    let git_repo = open_source(&root, folder)?;
    // Honor the reinforcement hook toggle when refreshing the graph.
    let lookback_days = ctx
        .cfg
        .reinforce
        .enabled
        .then_some(ctx.cfg.reinforce.search_edit_days);
    let conn = ctx.conn()?;
    prepare_source(conn, &mut req, &root, git_repo.as_ref(), folder)?;
    let outcome = index_repo(
        conn,
        &req,
        &root,
        git_repo.as_ref(),
        lookback_days,
        sink,
        (excluded.as_deref(), max_bytes),
    )
    .and_then(|response| {
        embedding::finish(
            conn,
            &req,
            &root,
            embed_cmd.as_deref(),
            &embed_model,
            sink,
            response,
        )
    });
    runner::record_run(conn, &req, &root, &started_at, started.elapsed(), &outcome);
    outcome
}

fn open_source(root: &Path, folder: bool) -> Result<Option<Repository>> {
    if !root.is_dir() {
        return Err(Error::Unavailable(format!(
            "code source is not a directory: {}",
            root.display()
        )));
    }
    if folder {
        Ok(None)
    } else {
        Ok(Some(Repository::open(root).map_err(map_git_err)?))
    }
}

fn prepare_source(
    conn: &Connection,
    req: &mut Request,
    root: &Path,
    git_repo: Option<&Repository>,
    folder: bool,
) -> Result<()> {
    if let Some(git_repo) = git_repo {
        redirect_worktree_label(conn, req, git_repo)?;
    }
    refuse_if_archived(conn, &req.repo)?;
    if folder
        && let Some(existing) = repo_marker::root_path(conn, &req.repo)?
        && Path::new(&existing).canonicalize().ok() != root.canonicalize().ok()
    {
        return Err(Error::Conflict(format!(
            "code source identity {} belongs to another root",
            req.repo
        )));
    }
    Ok(())
}

/// Redirect new linked-worktree labels to the main repository's label, including
/// labels supplied by old installed hooks. Preserve existing custom/legacy rows
/// so an established index is never silently repointed to another checkout.
fn redirect_worktree_label(
    conn: &Connection,
    req: &mut Request,
    git_repo: &Repository,
) -> Result<()> {
    if repo_marker::exists(conn, &req.repo)? || !git_utils::is_linked_worktree(git_repo) {
        return Ok(());
    }
    let Some(main_label) = git_utils::repo_label(git_repo) else {
        return Ok(());
    };
    if main_label == req.repo {
        return Ok(());
    }
    tracing::warn!(
        requested = %req.repo,
        repo = %main_label,
        "index-code: requested label is a linked worktree directory, not a repository; \
         indexing under the main worktree's label instead (run `comemory install-hooks` \
         to refresh a git hook that predates this rule)",
    );
    req.repo = main_label;
    Ok(())
}

/// Record canonical repositories at their durable main worktree. Custom and
/// legacy labels retain their walked root to preserve their existing index.
fn recorded_root_for(repo: &str, walked: &Path, git_repo: &Repository) -> PathBuf {
    if git_utils::repo_label(git_repo).as_deref() != Some(repo) {
        return walked.to_path_buf();
    }
    git_utils::main_worktree_dir(git_repo).unwrap_or_else(|| walked.to_path_buf())
}

/// `Err(Error::BadRequest)` when `repo` carries `repo_marker.archived = 1`
/// (`POST /api/v1/repos/{name}/archive`): an archived repo keeps its
/// memories searchable but is never re-indexed, by any entry point — the
/// HTTP routes pre-check this for a fast `400`, and [`run_with_progress`]
/// re-checks it so the CLI and a job body honor the flag too. An unknown
/// repo is fine — the first run for a label is how it becomes known.
pub fn refuse_if_archived(conn: &Connection, repo: &str) -> Result<()> {
    if repo_marker::archived(conn, repo)?.unwrap_or(false) {
        return Err(Error::BadRequest(format!(
            "repo {repo} is archived; un-archive it before indexing"
        )));
    }
    Ok(())
}

/// The walk proper: mirror the repo's symbols in one transaction, then run
/// the best-effort graph post-pass.
fn index_repo(
    conn: &mut Connection,
    req: &Request,
    root: &Path,
    git_repo: Option<&Repository>,
    lookback_days: Option<u32>,
    sink: Option<&dyn ProgressSink>,
    scope: (Option<&Path>, u64),
) -> Result<Response> {
    let mut imports_by_file: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // `BEGIN IMMEDIATE`, not the default deferred begin: this transaction
    // READS first (`ensure_repo_format`) and only writes symbol rows once
    // the walk is under way. A deferred begin takes a read snapshot at that
    // first read, and any other connection that commits before the walk's
    // first write — the search path's telemetry write is one, and it runs
    // concurrently with a `POST /api/v1/code/index` job by design — makes
    // the upgrade fail with `SQLITE_BUSY` *immediately*. `busy_timeout`
    // cannot wait that case out, so the job died as `store_locked`.
    let tx = write_transaction(conn)?;
    let previous = runner::prepare_index(&tx, req, root)?;
    let (files_indexed, unchanged, removed, unsupported) = runner::walk_repo(
        &tx,
        &req.repo,
        root,
        git_repo,
        &mut imports_by_file,
        sink,
        (&previous, scope.0, scope.1),
    )?;
    code_row::stamp_repo_format(&tx, &req.repo)?;
    let recorded_root = git_repo.map_or_else(
        || root.to_path_buf(),
        |git| recorded_root_for(&req.repo, root, git),
    );
    walk::stamp_repo_root(&tx, &req.repo, &recorded_root)?;
    stamp_last_indexed(&tx, &req.repo, root);
    tx.commit()?;
    runner::refresh_graph(
        conn,
        req,
        root,
        &imports_by_file,
        lookback_days,
        git_repo.is_some(),
    );
    Ok(Response {
        repo: req.repo.clone(),
        files_indexed,
        mode: req.mode,
        embedded: 0,
        embedding_failed: 0,
        embedding_pending: 0,
        unchanged,
        removed,
        unsupported,
    })
}

/// Best-effort stamp of `repo_marker.last_head`/`last_indexed_at` after a
/// successful walk, so `domains::code::repos`'s freshness comparison
/// (`src/domains/code/repos/git_state.rs`) has a HEAD to compare against. A
/// HEAD-resolution failure (e.g. an unborn branch) or write failure only
/// costs freshness tracking — the symbol rows this transaction already
/// built are not rolled back for it.
fn stamp_last_indexed(tx: &Connection, repo: &str, root: &Path) {
    match git_utils::current_head(root) {
        Ok(head) => {
            if let Err(e) = code_row::upsert_last_indexed(tx, repo, &head) {
                tracing::warn!(repo = %repo, error = %e, "index-code: last_head stamp failed");
            }
        }
        Err(e) => {
            tracing::warn!(
                repo = %repo,
                error = %e,
                "index-code: could not resolve HEAD to stamp last_head",
            );
        }
    }
}

#[cfg(test)]
#[path = "tests/index_code.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/index_code_freshness.rs"]
mod freshness_tests;
