//! `repos::{Request, Response, run}` — the shared middle of
//! `comemory repos` / `GET /api/v1/repos`: the indexed code-repository
//! inventory the console's Repositories screen and the Code graph's repo
//! legend need.
//!
//! Joins `repo_marker` (`repo`, `root_path`, `last_head`, `last_indexed_at`)
//! with per-repo counters over `indexed_files`, `code_symbols`, and
//! `memories`, then resolves each row's git freshness
//! ([`git_state::resolve`]) against the real working tree on disk. Split
//! for the size ceiling: this file owns the SQL join and the row shape,
//! [`git_state`] owns the HEAD comparison, remote/branch lookup, and
//! changed-file count — and never returns an error (see its module doc).
//!
//! A repo a peer shared has no `repo_marker` row here until this machine
//! indexes a checkout of it, so the inventory also reads
//! [`crate::domains::code::remote_view::repos`]. The two sides merge by
//! label: a repo this machine indexed AND a peer shared is one row carrying
//! both revisions, never two rows for one repository.
//!
//! **Must-not-create-the-db invariant** (the same rule `maintenance::stats` keeps):
//! a read command must not create and migrate a database as a side effect
//! of being asked which repos are indexed. On a data dir with no
//! `comemory.db`, `run` never calls [`Ctx::conn`] and reports an empty
//! inventory.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::domains::code::remote_view;
use crate::prelude::*;
use crate::store::remote_code_view::SharedRepo;
use crate::store::repos_inventory::{self, RepoMarkerRow};
use crate::utilities::context::Ctx;

/// The git-state half of a row: HEAD comparison, remote/branch lookup, and
/// changed-file count.
pub mod git_state;

/// `comemory repos` / `GET /api/v1/repos` request.
#[derive(Deserialize, Debug, Default, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Narrow the inventory to one repo label.
    #[serde(default)]
    pub repo: Option<String>,
}

/// One indexed repo's inventory row.
#[derive(Serialize, Debug)]
pub struct Row {
    /// The repo label, as stored on `code_symbols.repo` / `memories.repo`.
    pub repo: String,
    /// The absolute working-tree root captured at index time
    /// (`repo_marker.root_path`); `None` for a pre-v7 repo or one whose
    /// root could not be canonicalized.
    pub root_path: Option<String>,
    /// `git remote.origin.url`, when the working tree is present and
    /// carries an `origin` remote.
    pub remote: Option<String>,
    /// The working tree's currently checked-out branch; `None` when
    /// detached, unborn, or the tree is unreadable.
    pub branch: Option<String>,
    /// Distinct paths in `indexed_files` for this repo.
    pub files: u64,
    /// Rows in `code_symbols` for this repo, including cAST chunk children.
    pub symbols: u64,
    /// Live memories (`deleted_at IS NULL`) filed under this repo label.
    pub memories: u64,
    /// The HEAD oid recorded by the last successful `index-code` run.
    pub last_head: Option<String>,
    /// Timestamp of the last successful `index-code` run.
    pub last_indexed_at: Option<String>,
    /// `"archived"` when [`Row::archived`] is set (it outranks every git
    /// state — an archived repo is not being indexed at all), else
    /// `"fresh"` (HEAD unchanged since the last index), `"stale"` (HEAD
    /// moved), or `"unknown"` (no root, no last index, or the working tree
    /// / git itself is unreadable). `GET /api/v1/repos` overlays a fourth
    /// value, `"indexing"`, when the job registry has a live run for this
    /// repo — see [`Row::indexing_job`].
    pub status: String,
    /// `git diff --name-only <last_head>..HEAD` count when `status ==
    /// "stale"`; `None` otherwise, and `None` on any git failure.
    pub changed_files: Option<u64>,
    /// `repo_marker.archived` — the console's "archive" action: stop
    /// indexing this repo, keep its memories searchable, delete nothing.
    /// `POST /api/v1/index/runs` refuses an archived repo and
    /// `cli::lazy_reindex` skips it.
    pub archived: bool,
    /// The id of the live `index-code` job for this repo, when one is
    /// queued or running. Only `GET /api/v1/repos` fills this in (from the
    /// server's job registry); the CLI has no job registry, so its rows
    /// omit the field entirely.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexing_job: Option<String>,
    /// The head of the generation a peer shared for this repo, when one is
    /// active here. Independent of [`Row::last_head`]: that is what THIS
    /// machine indexed, this is what a peer did, and a repo can carry both
    /// at once — or only this one, on a machine with no checkout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_head: Option<String>,
    /// Files in the shared manifest, when a shared generation is active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_files: Option<u64>,
}

/// The indexed-repository inventory as emitted under `--json` and in the
/// `/api/v1/repos` `data` field.
#[derive(Serialize, Debug)]
pub struct Response {
    /// One row per `repo_marker` entry, ordered by repo label.
    pub repos: Vec<Row>,
}

/// Collect the inventory. See the module doc for why a missing database is
/// reported as an empty list rather than created.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Response> {
    if !ctx.paths.db_path().exists() {
        return Ok(Response { repos: Vec::new() });
    }
    let conn = ctx.conn()?;
    let markers = repos_inventory::fetch(conn, req.repo.as_deref())?;
    let mut shared: BTreeMap<String, SharedRepo> = remote_view::repos(conn)?
        .into_iter()
        .filter(|s| req.repo.as_ref().is_none_or(|want| *want == s.repo))
        .map(|s| (s.repo.clone(), s))
        .collect();
    let mut repos: Vec<Row> = markers
        .into_iter()
        .map(|m| {
            let shared = shared.remove(&m.repo);
            build_row(m, shared.as_ref())
        })
        .collect();
    // What is left is shared-only: a repo this machine has never indexed, so
    // it has no marker to join against and would otherwise be invisible.
    repos.extend(shared.into_values().map(shared_only_row));
    repos.sort_by(|a, b| a.repo.cmp(&b.repo));
    Ok(Response { repos })
}

/// A repo known only because a peer shared it: the same row shape over an
/// empty marker — no root, no local head, no counters — reported as
/// `"shared"` rather than a git freshness this machine cannot resolve
/// without a working tree.
fn shared_only_row(shared: SharedRepo) -> Row {
    let marker = RepoMarkerRow {
        repo: shared.repo.clone(),
        root_path: None,
        last_head: None,
        last_indexed_at: None,
        files: 0,
        symbols: 0,
        memories: 0,
        archived: false,
    };
    Row {
        status: "shared".to_string(),
        ..build_row(marker, Some(&shared))
    }
}

/// Resolve one [`RepoMarkerRow`] into its final [`Row`], filling in the
/// git-derived fields via [`git_state::resolve`] and the shared revision when
/// a peer's generation is active for the same label.
fn build_row(m: RepoMarkerRow, shared: Option<&SharedRepo>) -> Row {
    let git = git_state::resolve(m.root_path.as_deref(), m.last_head.as_deref());
    Row {
        repo: m.repo,
        root_path: m.root_path,
        remote: git.remote,
        branch: git.branch,
        files: m.files,
        symbols: m.symbols,
        memories: m.memories,
        last_head: m.last_head,
        last_indexed_at: m.last_indexed_at,
        status: if m.archived {
            "archived".to_string()
        } else {
            git.status.to_string()
        },
        changed_files: git.changed_files,
        archived: m.archived,
        indexing_job: None,
        shared_head: shared.map(|s| s.head.clone()),
        shared_files: shared.map(|s| u64::try_from(s.file_count).unwrap_or(0)),
    }
}

#[cfg(test)]
#[path = "tests/repos.rs"]
mod tests;
