//! `api::repos::{Request, Response, run}` — the shared middle of
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
//! **Must-not-create-the-db invariant** (the same rule `api::stats` keeps):
//! a read command must not create and migrate a database as a side effect
//! of being asked which repos are indexed. On a data dir with no
//! `comemory.db`, `run` never calls [`Ctx::conn`] and reports an empty
//! inventory.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::prelude::*;

/// The git-state half of a row: HEAD comparison, remote/branch lookup, and
/// changed-file count.
pub mod git_state;

/// `comemory repos` / `GET /api/v1/repos` request.
#[derive(Deserialize, Debug, Default)]
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
    let markers = fetch_markers(conn, req.repo.as_deref())?;
    let repos = markers.into_iter().map(build_row).collect();
    Ok(Response { repos })
}

/// One `repo_marker` row plus its joined counters, before git resolution.
struct Marker {
    repo: String,
    root_path: Option<String>,
    last_head: Option<String>,
    last_indexed_at: Option<String>,
    files: u64,
    symbols: u64,
    memories: u64,
    archived: bool,
}

/// Join `repo_marker` against the per-repo counters, narrowed to `repo`
/// when one was requested, ordered by repo label for deterministic output.
///
/// Every counter subquery carries an alias and the mapper reads BY NAME:
/// adding a column, or moving one, then cannot silently shift what each
/// field is filled from.
fn fetch_markers(conn: &Connection, repo: Option<&str>) -> Result<Vec<Marker>> {
    let sql = "SELECT rm.repo, rm.root_path, rm.last_head, rm.last_indexed_at, rm.archived, \
                      (SELECT COUNT(DISTINCT path) FROM indexed_files WHERE repo = rm.repo) AS files, \
                      (SELECT COUNT(*) FROM code_symbols WHERE repo = rm.repo) AS symbols, \
                      (SELECT COUNT(*) FROM memories WHERE repo = rm.repo AND deleted_at IS NULL) \
                        AS memories \
               FROM repo_marker rm \
               WHERE (?1 IS NULL OR rm.repo = ?1) \
               ORDER BY rm.repo";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([repo], |r| {
        Ok(Marker {
            repo: r.get("repo")?,
            root_path: r.get("root_path")?,
            last_head: r.get("last_head")?,
            last_indexed_at: r.get("last_indexed_at")?,
            archived: r.get::<_, i64>("archived")? != 0,
            files: r.get::<_, i64>("files")? as u64,
            symbols: r.get::<_, i64>("symbols")? as u64,
            memories: r.get::<_, i64>("memories")? as u64,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Resolve one `Marker` into its final [`Row`], filling in the git-derived
/// fields via [`git_state::resolve`].
fn build_row(m: Marker) -> Row {
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
    }
}

#[cfg(test)]
#[path = "tests/repos.rs"]
mod tests;
