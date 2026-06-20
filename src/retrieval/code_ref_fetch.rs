//! Per-repo current-state lookups behind code-ref freshness.
//!
//! Turns a pinned `code_ref` into a [`RefStatus`] by gathering the live signals
//! [`classify`] needs: the repo's working-tree root (reusing
//! [`crate::serve::repo_root::resolve_root`], `Err` → repo not on disk), the
//! current HEAD-tree blob of the referenced file, and — only when the index is
//! current for that repo — whether the symbol still resolves. Repo-level facts
//! (root, currency) are cached so a bundle citing many refs in one repo pays
//! the git/DB cost once.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension};

use crate::retrieval::code_ref_status::{CurrentRef, RefStatus, classify};
use crate::serve::repo_root::resolve_root;

/// Cached per-repo facts: the resolved working-tree root (`None` when the repo
/// is not on disk) and whether the code index is current for it.
struct RepoState {
    root: Option<PathBuf>,
    index_current: bool,
}

/// Repo-keyed cache so each repo's root + currency resolves once per bundle.
#[derive(Default)]
pub struct RefStatusCache {
    repos: HashMap<String, RepoState>,
}

impl RefStatusCache {
    /// Classify one code ref. `resolved` is whether the symbol address matched
    /// a live `code_symbols` row; it only informs symbol-ghost when the index
    /// is current (else `symbol_present` degrades to `None` → `Unknown`).
    pub fn status(
        &mut self,
        conn: &Connection,
        repo: &str,
        path: &str,
        is_symbol: bool,
        pinned_blob: Option<&str>,
        resolved: bool,
    ) -> RefStatus {
        let state = self.repo_state(conn, repo);
        let head_blob = head_blob_for(state.root.as_deref(), path);
        let symbol_present = if is_symbol && state.index_current {
            Some(resolved)
        } else {
            None
        };
        let cur = CurrentRef {
            head_blob: head_blob.as_deref(),
            repo_on_disk: state.root.is_some(),
            symbol_present,
        };
        classify(pinned_blob, &cur, is_symbol)
    }

    /// Resolve (and cache) the repo's root + index-currency.
    fn repo_state(&mut self, conn: &Connection, repo: &str) -> &RepoState {
        self.repos
            .entry(repo.to_string())
            .or_insert_with(|| repo_state_uncached(conn, repo))
    }
}

/// Compute a [`RepoState`] from scratch (root via `resolve_root`, currency via
/// the lazy-reindex HEAD comparison). Split out so the cache `entry` closure
/// stays a one-liner.
fn repo_state_uncached(conn: &Connection, repo: &str) -> RepoState {
    let root = resolve_root(conn, repo, &HashMap::new()).ok();
    let index_current = root
        .as_deref()
        .map(|r| index_is_current(conn, repo, r))
        .unwrap_or(false);
    RepoState {
        root,
        index_current,
    }
}

/// HEAD-tree blob of `path` in `root`; `None` when the root is unknown, the
/// file is gone from HEAD, or git could not read it (degrades to ghost/unknown
/// downstream rather than erroring the whole bundle).
fn head_blob_for(root: Option<&Path>, path: &str) -> Option<String> {
    let root = root?;
    crate::git_utils::blob_oid_at_head(root, path)
        .ok()
        .flatten()
}

/// Whether the code index reflects the repo's current HEAD: the lazy-reindex
/// signal — `repo_marker.last_mined_commit` equals `git_utils::current_head`.
/// A missing marker, unborn HEAD, or read error is treated as not-current.
fn index_is_current(conn: &Connection, repo: &str, root: &Path) -> bool {
    let head = match crate::git_utils::current_head(root) {
        Ok(head) => head,
        Err(e) => {
            tracing::debug!(repo, error = %e, "current_head failed; treating index as stale");
            return false;
        }
    };
    let last: Option<String> = conn
        .query_row(
            "SELECT last_mined_commit FROM repo_marker WHERE repo = ?1",
            [repo],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
        .ok()
        .flatten()
        .flatten();
    last.as_deref() == Some(head.as_str())
}
