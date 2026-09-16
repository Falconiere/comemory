#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Add a linked `git worktree` to a real test repo, for the tests that pin
//! the rule "a checkout files under its MAIN worktree's basename"
//! (`git_utils::repo_label`). Shells out to the real `git` so the worktree
//! has the on-disk layout users get — a `.git` FILE pointing at
//! `<main>/.git/worktrees/<name>`, which is exactly what a naive
//! `<root>/.git/hooks` join or a `workdir()` basename gets wrong.

use std::path::Path;

/// `git worktree add -b <branch> <dest>` from `main` (which must already
/// have a commit — a worktree cannot be added from an unborn HEAD).
pub fn add_worktree(main: &Path, dest: &Path, branch: &str) {
    let dest = dest.to_str().expect("utf8 worktree path");
    super::git_repo::run_git(main, &["worktree", "add", "-q", "-b", branch, dest]);
}
