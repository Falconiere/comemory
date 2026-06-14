//! Build minimal real git repos in tempdirs for integration tests.
//!
//! Everything shells out to the real `git` binary (rather than driving
//! `git2` directly) so tests exercise the same on-disk layout a user repo
//! would have — including the `.git/` directory `Repository::discover`
//! and `Repository::open` look for.
//!
//! This base file carries the repo-bootstrap primitives (`run_git`,
//! `init_repo`). The commit and sample-repo builders live in the sibling
//! `git_commit.rs` / `git_sample.rs` helpers so each flat test binary can
//! `#[path]`-include only the pieces it actually calls — every `pub fn` a
//! binary pulls in is then exercised, keeping `-D warnings` (dead_code)
//! green without `#[allow]`.

use std::path::Path;
use std::process::Command;

/// Run a git subcommand in `repo`, panicking on failure — the test
/// environment is broken if `git` itself cannot succeed.
pub fn run_git(repo: &Path, args: &[&str]) {
    let st = Command::new("git")
        .current_dir(repo)
        .args(args)
        .status()
        .expect("invoke git");
    assert!(st.success(), "git {args:?}");
}

/// Initialise `repo` (created if missing) as a git repository with a local
/// test identity and a pinned `main` branch, so commits succeed on CI hosts
/// without a global identity and behaviour is stable across git versions
/// that default to either `master` or `main`. No commit is made.
pub fn init_repo(repo: &Path) {
    std::fs::create_dir_all(repo).expect("create repo dir");
    run_git(repo, &["init", "-q"]);
    run_git(repo, &["config", "user.email", "test@example.com"]);
    run_git(repo, &["config", "user.name", "test"]);
    run_git(repo, &["checkout", "-q", "-b", "main"]);
}
