//! Keep every *hooked* repo's code index fresh from any cwd — the local half
//! of `comemory sync --action auto`. A repo is hooked when one of
//! [`GIT_HOOKS`] carries comemory's marker. [`index_checkout`] indexes the
//! checkout a hook fired in (step 1); [`refresh_stale`] re-indexes every other
//! registered repo whose HEAD moved (step 2). Both run inside one sync pass,
//! under its lock. A repo's index failure is counted, never fatal; only store
//! reads propagate.

use std::path::{Path, PathBuf};

use git2::Repository;
use serde::Serialize;

use crate::config::{Config, Paths};
use crate::domains::code::git_utils::{self, CheckoutKind};
use crate::domains::code::hooks::GIT_HOOKS;
use crate::domains::code::index_code::{self, IndexMode};
use crate::domains::code::reindex_policy::same_root;
use crate::prelude::*;
use crate::store::{Connection, repo_marker};
use crate::utilities::context::Ctx;

/// What one pass's refresh did, reported by `comemory sync` under
/// `refresh`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RefreshStats {
    /// Checkouts considered: the triggering checkout, plus every hooked
    /// registered repo that could be refreshed at all (on disk, a main
    /// checkout, not archived, HEAD born).
    pub checked: u32,
    /// Checkouts actually re-indexed.
    pub refreshed: u32,
    /// Checkouts whose index run failed; `errors` names each.
    pub failed: u32,
    /// One line per failure, `<label or path>: <error>`.
    pub errors: Vec<String>,
}

impl RefreshStats {
    /// Count one failed checkout and keep its `<who>: <error>` line.
    pub fn record_failure(&mut self, who: &str, e: impl std::fmt::Display) {
        self.failed += 1;
        self.errors.push(format!("{who}: {e}"));
    }
}

/// A checkout a git hook named: the label it indexes under and the working
/// tree to walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    /// The main worktree's basename ([`git_utils::repo_label`]) — the same
    /// label from a linked worktree as from the main checkout.
    pub label: String,
    /// The checkout's own working-tree root, canonicalized.
    pub root: PathBuf,
}

/// Resolve `path` (any directory inside a work tree) to its [`Checkout`];
/// `None` outside a git work tree, on a bare repository, or when the main
/// worktree's name is not UTF-8.
///
/// A workdir git just opened that still fails to canonicalize (permissions,
/// a symlink loop) is kept as git reported it, and the failure is logged:
/// at worst the trigger then runs its own incremental pass instead of
/// coalescing, whereas dropping the checkout would skip its index entirely.
pub fn resolve_checkout(path: &Path) -> Option<Checkout> {
    let git = Repository::discover(path).ok()?;
    let label = git_utils::repo_label(&git)?;
    let workdir = git.workdir()?;
    let root = workdir.canonicalize().unwrap_or_else(|e| {
        tracing::warn!(
            workdir = %workdir.display(),
            error = %e,
            "hooked refresh: workdir did not canonicalize; using it as git reported it",
        );
        workdir.to_path_buf()
    });
    Some(Checkout { label, root })
}

/// Whether a later pass's [`refresh_stale`] would re-index `checkout` on its
/// own: its label is registered, at this very root, and not archived. The
/// sync pass lets a trigger for such a checkout coalesce into an already
/// queued pass; any other checkout has to run its own.
///
/// # Errors
/// Store read failures.
pub fn swept_by_a_later_pass(conn: &Connection, checkout: &Checkout) -> Result<bool> {
    if repo_marker::archived(conn, &checkout.label)?.unwrap_or(false) {
        return Ok(false);
    }
    Ok(repo_marker::root_path(conn, &checkout.label)?
        .is_some_and(|root| same_root(&root, &checkout.root)))
}

/// Step 1: index the checkout a hook fired in. An archived label is left
/// alone (it is deliberately no longer indexed); any index failure is
/// counted in `stats`.
///
/// # Errors
/// Store read failures before the index run.
pub fn index_checkout(
    paths: &Paths,
    cfg: &Config,
    conn: &mut Connection,
    checkout: &Checkout,
    stats: &mut RefreshStats,
) -> Result<()> {
    if repo_marker::archived(conn, &checkout.label)?.unwrap_or(false) {
        return Ok(());
    }
    stats.checked += 1;
    index(paths, cfg, conn, &checkout.label, &checkout.root, stats);
    Ok(())
}

/// Step 2: re-index every registered, hooked repo whose HEAD moved since its
/// last index, except `skip` (the label step 1 just indexed).
///
/// A row is passed over, untouched, when it is archived, when its recorded
/// root is gone, is not a git checkout, or is a linked worktree (the same
/// rows a code push withholds), when none of its hooks is comemory's, or
/// when its HEAD is unborn or already indexed.
///
/// # Errors
/// Store read failures.
pub fn refresh_stale(
    paths: &Paths,
    cfg: &Config,
    conn: &mut Connection,
    skip: Option<&str>,
    stats: &mut RefreshStats,
) -> Result<()> {
    for label in repo_marker::all_repos(conn)? {
        if skip == Some(label.as_str()) || repo_marker::archived(conn, &label)?.unwrap_or(false) {
            continue;
        }
        let Some(root) = repo_marker::root_path(conn, &label)?.map(PathBuf::from) else {
            continue;
        };
        if !hooked_main_checkout(&root) {
            continue;
        }
        let Ok(head) = git_utils::current_head(&root) else {
            continue;
        };
        stats.checked += 1;
        if repo_marker::last_head(conn, &label)?.as_deref() == Some(head.as_str()) {
            continue;
        }
        index(paths, cfg, conn, &label, &root, stats);
    }
    Ok(())
}

/// Whether `root` is on disk, a repository's main working tree, and carries
/// at least one comemory hook.
fn hooked_main_checkout(root: &Path) -> bool {
    root.exists()
        && git_utils::checkout_kind(root) == CheckoutKind::Main
        && GIT_HOOKS
            .iter()
            .any(|hook| git_utils::hook_installed(root, hook))
}

/// One incremental `index-code` run, its outcome folded into `stats`.
fn index(
    paths: &Paths,
    cfg: &Config,
    conn: &mut Connection,
    label: &str,
    root: &Path,
    stats: &mut RefreshStats,
) {
    let mut ctx = Ctx::borrowed(paths, cfg, conn);
    let req = index_code::Request {
        repo: label.to_string(),
        path: root.to_string_lossy().into_owned(),
        mode: IndexMode::Incremental,
    };
    match index_code::run(&mut ctx, req) {
        Ok(_) => stats.refreshed += 1,
        Err(e) => {
            tracing::warn!(repo = %label, error = %e, "hooked refresh: index-code failed");
            stats.record_failure(label, e);
        }
    }
}

#[cfg(test)]
#[path = "tests/hooked_refresh.rs"]
mod tests;
