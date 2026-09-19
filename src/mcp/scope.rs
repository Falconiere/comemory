//! The session's default repo scope, and the per-call resolution rule.
//!
//! The scope key is [`repo_label_at`]'s label — the MAIN worktree's basename,
//! the same value the shell wrapper's `comemory_repo_key` computes — so a
//! linked worktree files under the repo it belongs to instead of minting a
//! second label.
//!
//! [`resolve`] is `serve::scope::RepoScope::resolve`'s rule without the HTTP
//! header: the session default is a DEFAULT, never an override.

use std::path::Path;

use crate::domains::code::git_utils::repo_label_at;

/// The session's default repo scope: an explicit `--repo` wins, else the repo
/// label of the working directory, else nothing (reads run unscoped and a
/// write is refused with `repo_required`).
///
/// A flag that is empty or all whitespace counts as absent, so
/// `--repo ""` behaves like no flag rather than pinning an unnamed scope.
pub fn default_repo(flag: Option<String>, cwd: &Path) -> Option<String> {
    non_empty(flag).or_else(|| repo_label_at(cwd))
}

/// The repo filter for one tool call: an explicit non-empty parameter wins,
/// otherwise the session default fills in. An empty or whitespace-only
/// parameter counts as absent — a client that sends `"repo": ""` for "no
/// opinion" gets the default, not an unscoped call.
pub fn resolve(param: Option<String>, default: Option<&str>) -> Option<String> {
    non_empty(param).or_else(|| default.map(str::to_string))
}

/// `Some(trimmed)` for a value with any non-whitespace content, else `None`.
fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
#[path = "tests/scope.rs"]
mod tests;
