#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Integration tests for `api::repos::git_state::resolve` against a REAL
//! git repo built with the git CLI (`crate::test_common::git_repo`). Every
//! degrade path (`None` root, a deleted working tree, an unstamped
//! `last_head`) is checked to land on `"unknown"` rather than panicking or
//! propagating — the module's whole contract (plan step 4: "git failure
//! degrades").

use crate::test_common::git_commit;
use crate::test_common::git_repo;

use std::path::Path;

use comemory::api::repos::git_state::resolve;
use comemory::git_utils::current_head;
use tempfile::TempDir;

#[test]
fn no_root_path_is_unknown() {
    let git = resolve(None, None);
    assert_eq!(git.status, "unknown");
    assert_eq!(git.remote, None);
    assert_eq!(git.branch, None);
    assert_eq!(git.changed_files, None);
}

#[test]
fn a_root_that_does_not_exist_on_disk_is_unknown() {
    let tmp = TempDir::new().expect("tempdir");
    let gone = tmp.path().join("never-existed");
    let git = resolve(Some(&gone.to_string_lossy()), Some("deadbeef"));
    assert_eq!(git.status, "unknown");
    assert_eq!(git.changed_files, None);
}

#[test]
fn a_deleted_working_tree_is_unknown() {
    let tmp = TempDir::new().expect("tempdir");
    let repo = tmp.path().join("repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(&repo, &[("a.txt", "one")], "init");
    let root = repo.to_string_lossy().into_owned();
    let head = current_head(&repo).expect("current_head");

    std::fs::remove_dir_all(&repo).expect("delete working tree");

    let git = resolve(Some(&root), Some(&head));
    assert_eq!(
        git.status, "unknown",
        "a deleted working tree must degrade to unknown, never panic or error out"
    );
    assert_eq!(git.changed_files, None);
}

#[test]
fn a_live_tree_with_no_stamped_last_head_is_unknown_but_still_reports_branch() {
    let tmp = TempDir::new().expect("tempdir");
    let repo = tmp.path().join("repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(&repo, &[("a.txt", "one")], "init");
    let root = repo.to_string_lossy().into_owned();

    let git = resolve(Some(&root), None);
    assert_eq!(
        git.status, "unknown",
        "no last_head to compare against is unknown, not fresh"
    );
    assert_eq!(
        git.branch,
        Some("main".to_string()),
        "branch is still resolvable even when freshness is unknown"
    );
    assert_eq!(git.changed_files, None);
}

#[test]
fn head_unchanged_since_last_index_is_fresh() {
    let tmp = TempDir::new().expect("tempdir");
    let repo = tmp.path().join("repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(&repo, &[("a.txt", "one")], "init");
    let root = repo.to_string_lossy().into_owned();
    let head = current_head(&repo).expect("current_head");

    let git = resolve(Some(&root), Some(&head));
    assert_eq!(git.status, "fresh");
    assert_eq!(
        git.changed_files, None,
        "a fresh repo has nothing to report as changed"
    );
}

#[test]
fn a_commit_since_last_index_is_stale_with_one_changed_file() {
    let tmp = TempDir::new().expect("tempdir");
    let repo = tmp.path().join("repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(&repo, &[("a.txt", "one")], "init");
    let root = repo.to_string_lossy().into_owned();
    let last_head = current_head(&repo).expect("current_head after init");

    git_commit::commit_files(&repo, &[("b.txt", "two")], "add b");

    let git = resolve(Some(&root), Some(&last_head));
    assert_eq!(git.status, "stale");
    assert_eq!(
        git.changed_files,
        Some(1),
        "exactly one file changed since last_head"
    );
}

#[test]
fn three_commits_touching_distinct_files_report_three_changed_files() {
    let tmp = TempDir::new().expect("tempdir");
    let repo = tmp.path().join("repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(&repo, &[("a.txt", "one")], "init");
    let root = repo.to_string_lossy().into_owned();
    let last_head = current_head(&repo).expect("current_head after init");

    git_commit::commit_files(&repo, &[("b.txt", "two")], "add b");
    git_commit::commit_files(&repo, &[("c.txt", "three")], "add c");
    git_commit::commit_files(&repo, &[("d.txt", "four")], "add d");

    let git = resolve(Some(&root), Some(&last_head));
    assert_eq!(git.status, "stale");
    assert_eq!(git.changed_files, Some(3));
}

#[test]
fn remote_and_branch_are_resolved_from_the_real_working_tree() {
    let tmp = TempDir::new().expect("tempdir");
    let repo = tmp.path().join("repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(&repo, &[("a.txt", "one")], "init");
    git_repo::run_git(
        &repo,
        &["remote", "add", "origin", "https://example.test/repo.git"],
    );
    let root = repo.to_string_lossy().into_owned();
    let head = current_head(&repo).expect("current_head");

    let git = resolve(Some(&root), Some(&head));
    assert_eq!(
        git.remote,
        Some("https://example.test/repo.git".to_string())
    );
    assert_eq!(git.branch, Some("main".to_string()));
}

/// Sanity: `resolve` must never panic given an arbitrary (non-git) path —
/// the "unreadable" degrade path must hold even for a directory that
/// exists but is not a git repo at all.
#[test]
fn a_directory_that_is_not_a_git_repo_is_unknown() {
    let tmp = TempDir::new().expect("tempdir");
    let not_a_repo: &Path = tmp.path();
    let git = resolve(Some(&not_a_repo.to_string_lossy()), Some("deadbeef"));
    assert_eq!(git.status, "unknown");
}
