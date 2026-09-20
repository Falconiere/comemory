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
//! Every indexed repo (`repo_marker`) is offered only when its current origin
//! resolves to an approved canonical GitHub identity. Its label can also match
//! `[sync] skip_repos`, `[sync] code_index` can be off, or its recorded root can
//! be invalid — a linked worktree, a root that is no longer on disk, or a
//! directory git cannot open ([`not_a_repository`]). A `git worktree add` is a second checkout
//! of a repository already synced under its own label; a row minted for one
//! before that rule existed kept standing a per-worktree "repository" up in
//! the console on every push. The unit of work is
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
use std::path::Path;

use crate::config::{Config, Paths};
use crate::domains::code::git_utils::{self, CheckoutKind};
use crate::domains::sync::AuthFile;
use crate::domains::sync::code_repo_push;
use crate::domains::sync::exchange::{CodeFileWire, CodeSymbolWire};
use crate::domains::sync::repository_policy::RepositoryPolicy;
use crate::prelude::*;
use crate::store::code_sync;
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
    /// Indexed repos withheld because their `origin` did not resolve to an
    /// approved canonical GitHub repository.
    pub blocked_repo: u32,
    /// Rows whose recorded root is a linked worktree — a second checkout of
    /// a repository already offered under its own label, never a repository
    /// of its own.
    pub skipped_worktree: u32,
    /// Rows whose recorded root is no longer a checkout `index-code` could
    /// open — gone from disk, or a leftover directory with no `.git` — so
    /// nothing could refresh the index they would push.
    pub skipped_missing_root: u32,
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
    let policy = RepositoryPolicy::load(conn, auth)?;
    let skip = cfg.sync.skip_matcher()?;
    for repo in repo_marker::all_repos(conn)? {
        if only.is_some_and(|wanted| wanted != repo) {
            continue;
        }
        if skip.is_skipped(&repo) {
            stats.skipped_config += 1;
            continue;
        }
        match not_a_repository(conn, &repo)? {
            Some(NotARepository::LinkedWorktree) => {
                tracing::warn!(
                    repo = %repo,
                    "code index push: this label's root is a linked worktree, not a \
                     repository; not offered (`comemory repos` lists it; disconnect it \
                     to drop the row)",
                );
                stats.skipped_worktree += 1;
                continue;
            }
            Some(reason @ (NotARepository::VanishedRoot | NotARepository::NoCheckout)) => {
                tracing::debug!(
                    repo = %repo,
                    reason = reason.label(),
                    "code index push: recorded root is not a checkout; not offered until it is",
                );
                stats.skipped_missing_root += 1;
                continue;
            }
            None => {}
        }
        let Some(canonical) = policy.code_repository(&repo) else {
            stats.blocked_repo += 1;
            continue;
        };
        if skip.is_skipped(canonical) {
            stats.skipped_config += 1;
            continue;
        }
        if let Err(e) =
            code_repo_push::push_repo(conn, auth, &policy, &repo, canonical, force, &mut stats)
        {
            tracing::warn!(repo = %repo, error = %e, "code index push failed");
            stats.failed += 1;
            stats.errors.push(format!("{repo}: {e}"));
        }
    }
    Ok(stats)
}

/// Why a `repo_marker` row names something other than a repository to offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotARepository {
    /// The recorded root is a linked worktree (`git worktree add`). Its
    /// files already reach the workspace under the main worktree's label —
    /// offering it too would stand one more "repository" up in the console
    /// for every checkout an agent ever made.
    LinkedWorktree,
    /// The recorded root is not on disk: a removed worktree, a deleted
    /// clone, or an unmounted volume. The first two must stop pushing; the
    /// third resumes on its own once the path is back, because nothing here
    /// deletes a row.
    VanishedRoot,
    /// The recorded root is a directory git cannot open as a checkout —
    /// what a removed worktree leaves behind when an ignored file survived
    /// it. `index-code` opens the root before anything else, so nothing
    /// could ever refresh the index this row would push.
    NoCheckout,
}

impl NotARepository {
    /// The short reason `comemory sync --action status` prints and serializes
    /// for a row it will not offer; the same words the push's counters use.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::LinkedWorktree => "worktree",
            Self::VanishedRoot => "missing_root",
            Self::NoCheckout => "no_checkout",
        }
    }
}

/// Classify `repo`'s `repo_marker` row against the working tree it records.
///
/// A row with no recorded root (pre-v7) has no checkout identity for policy
/// resolution and is withheld before this classifier is reached.
///
/// # Errors
/// Store failures reading `repo_marker.root_path`.
pub fn not_a_repository(conn: &Connection, repo: &str) -> Result<Option<NotARepository>> {
    let Some(root) = repo_marker::root_path(conn, repo)? else {
        return Ok(None);
    };
    let root = Path::new(&root);
    if !root.exists() {
        return Ok(Some(NotARepository::VanishedRoot));
    }
    Ok(match git_utils::checkout_kind(root) {
        CheckoutKind::Main => None,
        CheckoutKind::Linked => Some(NotARepository::LinkedWorktree),
        CheckoutKind::None => Some(NotARepository::NoCheckout),
    })
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

#[cfg(test)]
#[path = "tests/code.rs"]
mod tests;
