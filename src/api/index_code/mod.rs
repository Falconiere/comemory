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
use std::path::PathBuf;

use git2::Repository;
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::git_utils::map_git_err;
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

/// Walk `req.path`, extract symbols from every supported source file, and
/// mirror them into `code_symbols` (+ `code_fts`).
///
/// The whole walk runs in one SQLite transaction, so a mid-walk failure
/// rolls back cleanly: no partial `code_symbols`/`code_fts` rows, no stale
/// `indexed_files` cursors, no duplicate symbol rows on a re-run.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Response> {
    let root = PathBuf::from(&req.path);
    let git_repo = Repository::open(&root).map_err(map_git_err)?;
    let lookback_days = ctx.cfg.reinforce.search_edit_days;
    let conn = ctx.conn()?;
    let mut imports_by_file: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let tx = conn.transaction()?;
    code_row::ensure_repo_format(&tx, &req.repo)?;
    let mut walker = WalkBuilder::new(&root);
    walker.standard_filters(true);
    let mut files_indexed = 0usize;
    for entry in walker.build().filter_map(std::result::Result::ok) {
        if entry.file_type().is_some_and(|t| t.is_file())
            && walk::index_file(
                &tx,
                &req.repo,
                &root,
                &git_repo,
                entry.path(),
                &mut imports_by_file,
            )?
        {
            files_indexed += 1;
        }
    }
    code_row::stamp_repo_format(&tx, &req.repo)?;
    walk::stamp_repo_root(&tx, &req.repo, &root)?;
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
