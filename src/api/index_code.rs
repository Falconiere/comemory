//! `api::index_code::{Request, Response, run}` — shared middle of
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

use git2::Repository;
use ignore::WalkBuilder;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::git_utils::{self, map_git_err};
use crate::graph::{derived, materialize};
use crate::prelude::*;
use crate::store::code_row;

/// File-walk / symbol-write internals, shared (in part) with
/// `cli::index_code`'s `--extract` path.
pub mod walk;

/// `comemory index-code` (DB-write path) / `POST /api/v1/code/index`
/// request. No `extract` field — see the module doc.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Repo label stored alongside each symbol row.
    pub repo: String,
    /// Root of the working tree to walk. Must live inside a git repo so
    /// blob OIDs are available for the incremental skip path.
    pub path: String,
}

/// `POST /api/v1/code/index` response. `cli::index_code::run` emits
/// nothing on success (silent, unchanged); the HTTP job form carries a
/// small real count instead of an empty object.
#[derive(Serialize, Debug)]
pub struct Response {
    /// The `req.repo` label this run indexed.
    pub repo: String,
    /// Files actually (re)indexed — those skipped by the blob-OID gate
    /// (no language, untracked, unchanged since the last run) don't count.
    pub files_indexed: usize,
}

/// Progress-reporting sink for a long `index-code` walk: [`ProgressSink::on_progress`]
/// after every candidate file (indexed or skipped alike) with the running
/// `(done, total)` file counts, [`ProgressSink::on_log`] for one
/// human-readable line per file actually (re)indexed. `serve::jobs::worker`
/// implements this over the job [`crate::serve::jobs::Registry`]; the CLI
/// never constructs one — [`run`] passes `None`, which is the "no-op" the
/// plan calls for. Implementations must be best-effort: neither method
/// returns a `Result`, so a reporting failure can only be handled (e.g.
/// `tracing::warn!`) inside the implementation itself, never by failing the
/// walk it instruments.
pub trait ProgressSink: Send + Sync {
    /// Report progress after processing one candidate file.
    fn on_progress(&self, done: u64, total: u64);
    /// Append one line to the job's log tail.
    fn on_log(&self, line: &str);
}

/// `comemory index-code` (DB-write path) / `POST /api/v1/code/index`: walk
/// `req.path` and mirror its symbols into `code_symbols` (+ `code_fts`)
/// with no progress reporting. See [`run_with_progress`] for the
/// progress-reporting form the job worker uses.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Response> {
    run_with_progress(ctx, req, None)
}

/// Same as [`run`], but reports progress and log lines through `sink` (see
/// [`ProgressSink`]) when one is given.
///
/// The whole walk runs in one SQLite transaction, so a mid-walk failure
/// rolls back cleanly: no partial `code_symbols`/`code_fts` rows, no stale
/// `indexed_files` cursors, no duplicate symbol rows on a re-run.
pub fn run_with_progress(
    ctx: &mut Ctx<'_>,
    req: Request,
    sink: Option<&dyn ProgressSink>,
) -> Result<Response> {
    let root = PathBuf::from(&req.path);
    let git_repo = Repository::open(&root).map_err(map_git_err)?;
    let lookback_days = ctx.cfg.reinforce.search_edit_days;
    let conn = ctx.conn()?;
    let mut imports_by_file: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let tx = conn.transaction()?;
    code_row::ensure_repo_format(&tx, &req.repo)?;
    let files_indexed = walk_repo(&tx, &req.repo, &root, &git_repo, &mut imports_by_file, sink)?;
    code_row::stamp_repo_format(&tx, &req.repo)?;
    walk::stamp_repo_root(&tx, &req.repo, &root)?;
    stamp_last_indexed(&tx, &req.repo, &root);
    tx.commit()?;
    // Best-effort graph post-pass: the symbol index is already durable, so
    // a failure here (e.g. an unborn HEAD) costs only freshness.
    if let Err(e) =
        materialize::materialize(conn, &root, &req.repo, &imports_by_file, lookback_days)
    {
        tracing::warn!(
            repo = %req.repo,
            error = %e,
            "index-code: graph materialization failed; symbol index kept",
        );
    }
    derived::refresh_derived_best_effort(conn);
    Ok(Response {
        repo: req.repo,
        files_indexed,
    })
}

/// Walk every candidate file under `root` (the same `ignore`-crate filters
/// [`run_with_progress`] always used), indexing each into `tx` via
/// [`walk::index_file`] and reporting progress through `sink`. Returns the
/// count actually (re)indexed — entries skipped by the blob-OID gate still
/// count toward progress but not toward this return value.
///
/// Entries are collected up front (rather than indexed lazily) so `total`
/// is known before the loop starts — the progress sink needs a fixed
/// denominator.
fn walk_repo(
    tx: &Connection,
    repo: &str,
    root: &Path,
    git_repo: &Repository,
    imports_by_file: &mut BTreeMap<String, Vec<String>>,
    sink: Option<&dyn ProgressSink>,
) -> Result<usize> {
    let mut walker = WalkBuilder::new(root);
    walker.standard_filters(true);
    let entries: Vec<_> = walker
        .build()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|t| t.is_file()))
        .collect();
    let total = entries.len() as u64;
    let mut files_indexed = 0usize;
    for (i, entry) in entries.iter().enumerate() {
        if walk::index_file(
            tx,
            repo,
            root,
            git_repo,
            entry.path(),
            imports_by_file,
            sink,
        )? {
            files_indexed += 1;
        }
        if let Some(sink) = sink {
            sink.on_progress(i as u64 + 1, total);
        }
    }
    Ok(files_indexed)
}

/// Best-effort stamp of `repo_marker.last_head`/`last_indexed_at` after a
/// successful walk, so `api::repos`'s freshness comparison
/// (`src/api/repos/git_state.rs`) has a HEAD to compare against. A
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
