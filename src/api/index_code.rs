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
use std::time::{Duration, Instant};

use git2::Repository;
use ignore::WalkBuilder;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::api::Ctx;
use crate::git_utils::{self, map_git_err};
use crate::graph::{derived, materialize};
use crate::prelude::*;
use crate::store::{code_row, index_runs, memory_row, random_id};

/// File-walk / symbol-write internals, shared (in part) with
/// `cli::index_code`'s `--extract` path.
pub mod walk;

/// How much of the repo an `index-code` run re-extracts.
#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum IndexMode {
    /// Only files whose blob OID moved since the last run (the default —
    /// today's behavior, unchanged).
    #[default]
    Incremental,
    /// Every file: the repo's `indexed_files` cursor is cleared first so
    /// the blob-OID gate re-extracts everything. Lossy by construction —
    /// re-extracting a file goes through `code_row::purge_file_symbols`,
    /// which deletes that file's `code_vec` rows (BYO vectors only the
    /// caller's embedder can recreate; re-run `ingest-code` afterwards) and
    /// replaces its `code_symbols` rows, resetting per-symbol
    /// `access_count` / `last_accessed`. `code_feedback` (keyed by
    /// `(repo, path, symbol)`, not by row id) survives.
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
    /// Root of the working tree to walk. Must live inside a git repo so
    /// blob OIDs are available for the incremental skip path.
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
    /// Files actually (re)indexed — those skipped by the blob-OID gate
    /// (no language, untracked, unchanged since the last run) don't count.
    pub files_indexed: usize,
    /// The mode this run used. `full` means every file was re-extracted, so
    /// the repo's `code_vec` rows are gone until the caller re-runs
    /// `ingest-code` (see [`IndexMode::Full`]).
    pub mode: IndexMode,
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
    /// Whether the caller asked this run to stop (`POST /jobs/{id}/cancel`).
    /// Polled at every file boundary; `true` makes the walk return
    /// [`Error::Cancelled`] and roll its transaction back. Defaults to
    /// `false` — a sink that cannot be cancelled never is.
    fn is_cancelled(&self) -> bool {
        false
    }
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
/// (or a cooperative cancel) rolls back cleanly: no partial
/// `code_symbols`/`code_fts` rows, no stale `indexed_files` cursors, no
/// duplicate symbol rows on a re-run. Every run — `ok`, `error`, or
/// `cancelled` — is recorded in `index_runs` afterwards ([`record_run`]),
/// so the console's run history and "last run" tile have a source; the
/// recording is best-effort and never turns an indexed repo into a failure.
pub fn run_with_progress(
    ctx: &mut Ctx<'_>,
    req: Request,
    sink: Option<&dyn ProgressSink>,
) -> Result<Response> {
    let started = Instant::now();
    let started_at = memory_row::iso_format(OffsetDateTime::now_utc())?;
    let root = PathBuf::from(&req.path);
    // Opened BEFORE the connection: a path that is not a git repo must fail
    // without creating `comemory.db` as a side effect (and with no DB there
    // is nothing to record the failed run into).
    let git_repo = Repository::open(&root).map_err(map_git_err)?;
    // `None` when the operator turned search→edit reinforcement off
    // (`comemory hooks --disable search-edit-reinforcement`), which is what
    // makes that toggle actually gate the behavior rather than only report it.
    let lookback_days = ctx
        .cfg
        .reinforce
        .enabled
        .then_some(ctx.cfg.reinforce.search_edit_days);
    let conn = ctx.conn()?;
    refuse_if_archived(conn, &req.repo)?;
    let outcome = index_repo(conn, &req, &root, &git_repo, lookback_days, sink);
    record_run(conn, &req, &root, &started_at, started.elapsed(), &outcome);
    outcome
}

/// `Err(Error::BadRequest)` when `repo` carries `repo_marker.archived = 1`
/// (`POST /api/v1/repos/{name}/archive`): an archived repo keeps its
/// memories searchable but is never re-indexed, by any entry point — the
/// HTTP routes pre-check this for a fast `400`, and [`run_with_progress`]
/// re-checks it so the CLI and a job body honor the flag too. An unknown
/// repo is fine — the first run for a label is how it becomes known.
pub fn refuse_if_archived(conn: &Connection, repo: &str) -> Result<()> {
    let archived: Option<i64> = conn
        .query_row(
            "SELECT archived FROM repo_marker WHERE repo = ?1",
            [repo],
            |r| r.get(0),
        )
        .optional()?;
    if archived.is_some_and(|flag| flag != 0) {
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
    git_repo: &Repository,
    lookback_days: Option<u32>,
    sink: Option<&dyn ProgressSink>,
) -> Result<Response> {
    let mut imports_by_file: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let tx = conn.transaction()?;
    code_row::ensure_repo_format(&tx, &req.repo)?;
    if req.mode == IndexMode::Full {
        // Forget every blob-OID cursor so `walk::index_file` re-extracts
        // each file; the per-file purge inside it replaces the old rows —
        // and with them the repo's BYO `code_vec` rows, which nothing here
        // can recreate (see `IndexMode::Full`).
        tracing::warn!(
            repo = %req.repo,
            "index-code --mode full: re-extracting every file drops the repo's BYO code \
             vectors and per-symbol access counters; re-run `ingest-code` afterwards",
        );
        tx.execute("DELETE FROM indexed_files WHERE repo = ?1", [&req.repo])?;
    }
    let files_indexed = walk_repo(&tx, &req.repo, root, git_repo, &mut imports_by_file, sink)?;
    code_row::stamp_repo_format(&tx, &req.repo)?;
    walk::stamp_repo_root(&tx, &req.repo, root)?;
    stamp_last_indexed(&tx, &req.repo, root);
    tx.commit()?;
    // Best-effort graph post-pass: the symbol index is already durable, so
    // a failure here (e.g. an unborn HEAD) costs only freshness.
    if let Err(e) = materialize::materialize(conn, root, &req.repo, &imports_by_file, lookback_days)
    {
        tracing::warn!(
            repo = %req.repo,
            error = %e,
            "index-code: graph materialization failed; symbol index kept",
        );
    }
    let _stale = derived::refresh_derived_best_effort(conn);
    Ok(Response {
        repo: req.repo.clone(),
        files_indexed,
        mode: req.mode,
    })
}

/// Insert this run's `index_runs` row from `outcome`. Best-effort: a write
/// failure is logged, never propagated — the symbols already landed (or
/// already rolled back), and a history row is not worth failing over.
fn record_run(
    conn: &Connection,
    req: &Request,
    root: &Path,
    started_at: &str,
    elapsed: Duration,
    outcome: &Result<Response>,
) {
    let (slug, error, files_indexed) = match outcome {
        Ok(resp) => ("ok", None, resp.files_indexed),
        Err(Error::Cancelled) => ("cancelled", None, 0),
        Err(e) => ("error", Some(e.to_string()), 0),
    };
    let symbols: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM code_symbols WHERE repo = ?1",
            [&req.repo],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let root_path = root.canonicalize().ok();
    let written = random_id::random_hex(8).and_then(|id| {
        let finished_at = memory_row::iso_format(OffsetDateTime::now_utc())?;
        index_runs::insert(
            conn,
            &index_runs::NewIndexRun {
                id: &id,
                repo: &req.repo,
                root_path: root_path.as_deref().map(Path::to_string_lossy).as_deref(),
                mode: req.mode.as_str(),
                started_at,
                finished_at: &finished_at,
                duration_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
                files_indexed: files_indexed as u64,
                symbols: u64::try_from(symbols).unwrap_or(0),
                outcome: slug,
                error: error.as_deref(),
            },
        )
    });
    if let Err(e) = written {
        tracing::warn!(repo = %req.repo, error = %e, "index-code: index_runs row not recorded");
    }
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
        if sink.is_some_and(ProgressSink::is_cancelled) {
            return Err(Error::Cancelled);
        }
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
