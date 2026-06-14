//! Commit builder for the git-repo test fixtures.
//!
//! Pairs with `git_repo.rs`: a binary that `#[path]`-includes this file as
//! `mod git_commit;` must also include `git_repo.rs` as `mod git_repo;`,
//! since `commit_files` drives commits through `crate::git_repo::run_git`.

use std::path::Path;

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
    crate::git_repo::run_git(repo, &["add", "-A"]);
    crate::git_repo::run_git(repo, &["commit", "-q", "-m", msg]);
}
