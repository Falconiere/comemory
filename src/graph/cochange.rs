//! Git co-change mining: files that change in the same commit are
//! behaviorally coupled. Mines bounded history into weighted pairs.
//!
//! Pure git I/O, no SQLite — the indexer persists the mined pairs into
//! the `edges` table separately. Merge commits are diffed against their
//! FIRST parent only (standard co-change practice: the second-parent
//! diff replays already-counted commits and would double-count pairs).

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use git2::{Commit, Oid, Repository, Sort};

use crate::git_utils::map_git_err;
use crate::prelude::*;

/// Most recent commits walked on a first (cursor-less) run.
pub const FIRST_RUN_COMMIT_LIMIT: usize = 1000;

/// Commits touching more than this many files are skipped — formatting
/// sweeps and renames carry no coupling signal.
pub const MEGA_COMMIT_FILE_CAP: usize = 20;

/// One mined undirected pair with its co-occurrence count. Paths are
/// repo-relative; the pair is stored in lexicographic order (a < b).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoChange {
    /// Lexicographically smaller path.
    pub a: String,
    /// Lexicographically larger path.
    pub b: String,
    /// Number of commits in which the pair changed together.
    pub count: u32,
}

/// Result of one mining pass: the counted pairs, the new cursor (HEAD
/// oid), and whether the *previous* cursor could not be resolved.
///
/// `cursor_lost` is the caller's signal to RESET the repo's accumulated
/// `co_changed` weights before applying `pairs`: a lost cursor means the
/// bounded walk re-counted history that earlier runs already accumulated
/// into the edges table, so adding the fresh counts on top would
/// double-count every pair that survived the rewrite.
#[derive(Debug)]
pub struct MineOutcome {
    /// Mined undirected pairs, sorted lexicographically by `(a, b)`.
    pub pairs: Vec<CoChange>,
    /// New cursor: the HEAD oid at mining time.
    pub cursor: String,
    /// True when `since` was `Some` but its commit object no longer
    /// exists (rebase/amend/force-push followed by gc, or a corrupted
    /// marker row) — the pass ran as a bounded first run instead.
    pub cursor_lost: bool,
}

/// Walk commits newer than `since` (exclusive; `None` = first run,
/// bounded by [`FIRST_RUN_COMMIT_LIMIT`]) and count co-changed pairs
/// among `known_files`. Returns a [`MineOutcome`] carrying the pairs,
/// the new cursor (HEAD oid), and the lost-cursor flag.
///
/// The cursor is resolved BEFORE the walk starts: when its commit object
/// still exists, `revwalk.hide` excludes it and all its ancestors (exact
/// and exclusive — equivalent to the naive "break on sight" for a linear
/// history, and correct for branched ones). When it cannot be resolved
/// (gc'd after a history rewrite, or garbage), waiting to "see" it would
/// walk uncapped to the root and re-count everything into the
/// accumulating edge weights — instead the pass degrades to a first run
/// ([`FIRST_RUN_COMMIT_LIMIT`] cap) and reports `cursor_lost` so the
/// caller resets the accumulated weights.
///
/// Per commit, the full changed-file footprint is measured first: a
/// commit touching more than [`MEGA_COMMIT_FILE_CAP`] files is skipped
/// entirely (a formatting sweep is noise even if only a few indexed
/// files are in it), and commits whose intersection with `known_files`
/// is smaller than 2 contribute nothing.
///
/// # Errors
/// * `repo_root` is not a git repository.
/// * HEAD is unborn (a repo with no commits) — callers treat the whole
///   mining pass as best-effort and skip it.
/// * Any underlying `git2` failure, flattened via
///   [`crate::git_utils::map_git_err`].
pub fn mine_cochange(
    repo_root: &Path,
    known_files: &HashSet<String>,
    since: Option<&str>,
) -> Result<MineOutcome> {
    let repo = Repository::open(repo_root).map_err(map_git_err)?;
    let cursor = crate::git_utils::head_oid(&repo)?;

    let mut walk = repo.revwalk().map_err(map_git_err)?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)
        .map_err(map_git_err)?;
    walk.push_head().map_err(map_git_err)?;

    let mut cursor_lost = false;
    if let Some(stop) = since {
        match Oid::from_str(stop)
            .ok()
            .filter(|oid| repo.find_commit(*oid).is_ok())
        {
            Some(oid) => walk.hide(oid).map_err(map_git_err)?,
            None => {
                tracing::warn!(
                    cursor = %stop,
                    "cochange: stored cursor unresolvable (history rewrite?); \
                     re-mining bounded history and signaling a weight reset",
                );
                cursor_lost = true;
            }
        }
    }
    // The first-run cap also applies when the cursor was lost — without
    // it the walk would run to the root.
    let capped = since.is_none() || cursor_lost;

    let mut counts: BTreeMap<(String, String), u32> = BTreeMap::new();
    // `walked` counts commits already walked (enumerate index),
    // including mega-skipped ones — the first-run bound caps the walk,
    // not the number of pair-contributing commits.
    for (walked, oid) in walk.enumerate() {
        let oid = oid.map_err(map_git_err)?;
        if capped && walked >= FIRST_RUN_COMMIT_LIMIT {
            break;
        }

        let commit = repo.find_commit(oid).map_err(map_git_err)?;
        let changed = commit_changed_paths(&repo, &commit)?;
        if changed.len() > MEGA_COMMIT_FILE_CAP {
            tracing::debug!(oid = %oid, files = changed.len(), "cochange: skipping mega-commit");
            continue;
        }
        let mut hit: Vec<&String> = changed
            .iter()
            .filter(|p| known_files.contains(*p))
            .collect();
        if hit.len() < 2 {
            continue;
        }
        hit.sort();
        for (i, a) in hit.iter().enumerate() {
            for b in &hit[i + 1..] {
                *counts.entry(((*a).clone(), (*b).clone())).or_insert(0) += 1;
            }
        }
    }

    let pairs = counts
        .into_iter()
        .map(|((a, b), count)| CoChange { a, b, count })
        .collect();
    Ok(MineOutcome {
        pairs,
        cursor,
        cursor_lost,
    })
}

/// Collect the new-side paths changed by `commit` against its FIRST
/// parent; root commits diff against the empty tree. Delegates the
/// delta walk to [`crate::git_utils::collect_diff_paths`] — the
/// rev-string-resolving [`crate::git_utils::changed_files`] cannot
/// serve a revwalk directly, but the underlying collection is shared.
/// `pub(crate)` so `retrieval::code_rerank::working_set` reuses the
/// same first-parent diff for its recent-commit window.
pub(crate) fn commit_changed_paths(repo: &Repository, commit: &Commit<'_>) -> Result<Vec<String>> {
    let tree = commit.tree().map_err(map_git_err)?;
    let parent_tree = if commit.parent_count() > 0 {
        let parent = commit.parent(0).map_err(map_git_err)?;
        Some(parent.tree().map_err(map_git_err)?)
    } else {
        None
    };
    crate::git_utils::collect_diff_paths(repo, parent_tree.as_ref(), &tree)
}
