//! Sample-repo builder for the git-repo test fixtures.
//!
//! Pairs with `git_repo.rs` + `git_commit.rs`: a binary that
//! `#[path]`-includes this file as `mod git_sample;` must also include
//! both `git_repo.rs` (`mod git_repo;`) and `git_commit.rs`
//! (`mod git_commit;`), since `build_sample_repo` composes
//! `crate::git_repo::init_repo` with `crate::git_commit::commit_files`.

use std::path::{Path, PathBuf};

/// Create `<root>/sample-repo` with a single `src.rs` file containing two
/// top-level Rust functions, initialise it as a git repo, and commit the
/// initial tree. Returns the working tree root.
pub fn build_sample_repo(root: &Path) -> PathBuf {
    let repo = root.join("sample-repo");
    crate::git_repo::init_repo(&repo);
    crate::git_commit::commit_files(
        &repo,
        &[("src.rs", "fn main() {}\nfn helper() {}\n")],
        "init",
    );
    repo
}
