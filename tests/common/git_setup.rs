//! Build a minimal real git repo in a tempdir for index-code tests.
//!
//! Returns the working tree root once `git init` + one commit have run, so
//! callers can hand the path straight to `comemory index-code --path …` and
//! exercise the incremental blob-oid skip logic on a real repository.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Create `<root>/sample-repo` with a single `src.rs` file containing two
/// top-level Rust functions, initialise it as a git repo, and commit the
/// initial tree. Returns the working tree root.
pub fn build_sample_repo(root: &Path) -> PathBuf {
    let repo = root.join("sample-repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    std::fs::write(repo.join("src.rs"), "fn main() {}\nfn helper() {}\n").expect("write src.rs");
    let run = |args: &[&str]| {
        let st = Command::new("git")
            .current_dir(&repo)
            .args(args)
            .status()
            .expect("invoke git");
        assert!(st.success(), "git {args:?}");
    };
    run(&["init", "-q"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "test"]);
    run(&["add", "."]);
    run(&["commit", "-q", "-m", "init"]);
    repo
}
