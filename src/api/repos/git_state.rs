//! The git half of `api::repos` — HEAD comparison, remote/branch lookup, and
//! changed-file count. Split out from `repos.rs` for the size ceiling
//! (Binding Rule 4).
//!
//! **Every git failure here degrades rather than propagates** (plan step
//! 4, "git failure degrades"): a deleted working tree, an unreadable
//! `.git`, an unborn HEAD, or any other git error resolves to `status:
//! "unknown"` and every other field `None` — never an `Err`. `comemory
//! repos` must always exit 0 against a resolvable database; a repo's git
//! state being unreadable is a fact about that repo's row, not a reason to
//! fail the whole inventory.

use std::path::Path;

use crate::git_utils;

/// One repo row's git-derived fields.
pub struct GitState {
    /// `git remote.origin.url`, when resolvable.
    pub remote: Option<String>,
    /// The working tree's currently checked-out branch.
    pub branch: Option<String>,
    /// `"fresh"` | `"stale"` | `"unknown"` — see [`resolve`].
    pub status: &'static str,
    /// Files changed since the last index; only populated when `status ==
    /// "stale"`.
    pub changed_files: Option<u64>,
}

/// A repo with no usable root or an unreadable working tree: every field
/// degrades to its empty value.
const UNKNOWN: GitState = GitState {
    remote: None,
    branch: None,
    status: "unknown",
    changed_files: None,
};

/// Resolve one repo's git state from its stored `repo_marker.root_path` and
/// `last_head`, against the real working tree on disk.
///
/// * `root_path: None` (a pre-v7 repo, or the root could not be
///   canonicalized at index time) → [`UNKNOWN`].
/// * The working tree is gone, unreadable, or its HEAD cannot be resolved
///   (e.g. unborn) → [`UNKNOWN`] — a git failure degrades, it never
///   propagates.
/// * `last_head` is `None` but the tree resolves fine (`index-code` has not
///   yet stamped a marker row) → `"unknown"`; freshness has nothing to
///   compare against.
/// * The current HEAD equals `last_head` → `"fresh"`.
/// * The current HEAD differs → `"stale"`, with `changed_files` set to the
///   `git diff --name-only <last_head>..HEAD` count (`None` if that diff
///   itself fails).
pub fn resolve(root_path: Option<&str>, last_head: Option<&str>) -> GitState {
    let Some(root) = root_path else {
        return UNKNOWN;
    };
    let root = Path::new(root);
    let Ok(current) = git_utils::current_head(root) else {
        return UNKNOWN;
    };
    let remote = git_utils::remote_url(root, "origin");
    let branch = git_utils::current_branch(root).ok().flatten();
    let Some(last) = last_head else {
        return GitState {
            remote,
            branch,
            status: "unknown",
            changed_files: None,
        };
    };
    if last == current {
        return GitState {
            remote,
            branch,
            status: "fresh",
            changed_files: None,
        };
    }
    let changed_files = git_utils::changed_files(root, last, &current)
        .ok()
        .map(|files| files.len() as u64);
    GitState {
        remote,
        branch,
        status: "stale",
        changed_files,
    }
}

#[cfg(test)]
#[path = "tests/git_state.rs"]
mod tests;
