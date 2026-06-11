//! Build minimal real git repos in tempdirs for integration tests.
//!
//! Everything shells out to the real `git` binary (rather than driving
//! `git2` directly) so tests exercise the same on-disk layout a user repo
//! would have — including the `.git/` directory `Repository::discover`
//! and `Repository::open` look for.

use std::path::{Path, PathBuf};
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

/// Write `files` (`(repo-relative path, content)` pairs, parent dirs
/// created as needed) and record them all in a single commit `msg`.
pub fn commit_files(repo: &Path, files: &[(&str, &str)], msg: &str) {
    for (path, content) in files {
        let full = repo.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(&full, content).expect("write file");
    }
    run_git(repo, &["add", "-A"]);
    run_git(repo, &["commit", "-q", "-m", msg]);
}

/// Create `<root>/sample-repo` with a single `src.rs` file containing two
/// top-level Rust functions, initialise it as a git repo, and commit the
/// initial tree. Returns the working tree root.
pub fn build_sample_repo(root: &Path) -> PathBuf {
    let repo = root.join("sample-repo");
    init_repo(&repo);
    commit_files(
        &repo,
        &[("src.rs", "fn main() {}\nfn helper() {}\n")],
        "init",
    );
    repo
}
