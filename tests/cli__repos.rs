#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `comemory repos` driven as a real subprocess (spec AC-3, AC-4, AC-4b,
//! AC-5).
//!
//! Every repo here is a real git repository built with the git CLI and
//! indexed through the real `comemory index-code`; freshness/staleness is
//! observed from the command's own `--json` output after real `git commit`s
//! and a real `rm -rf` of a working tree — no mocks (Binding Rule 9).

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

use std::path::{Path, PathBuf};
use std::process::Command;

use assert_cmd::prelude::*;
use tempfile::TempDir;

/// Run `comemory` with the temp data dir, returning stdout on success.
fn comemory(data_dir: &Path, args: &[&str]) -> String {
    let out = Command::cargo_bin("comemory")
        .unwrap()
        .args(args)
        .env("COMEMORY_DATA_DIR", data_dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "comemory {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn repos_json(data_dir: &Path) -> serde_json::Value {
    let out = comemory(data_dir, &["repos", "--json"]);
    serde_json::from_str(&out).expect("repos --json parses")
}

/// Find the row for `repo` in a `repos --json` payload, panicking (with the
/// full payload) when it is missing so a failure is easy to diagnose.
fn row<'a>(repos: &'a serde_json::Value, repo: &str) -> &'a serde_json::Value {
    repos["repos"]
        .as_array()
        .expect("repos array")
        .iter()
        .find(|r| r["repo"].as_str() == Some(repo))
        .unwrap_or_else(|| panic!("no row for repo {repo:?} in {repos}"))
}

/// Build a real one-file git repo at `<root>/<name>` and return its path.
fn build_repo(root: &Path, name: &str, file: &str, body: &str) -> PathBuf {
    let repo = root.join(name);
    git_repo::init_repo(&repo);
    git_commit::commit_files(&repo, &[(file, body)], "init");
    repo
}

/// Index `repo_path` under `repo_label` via the real `comemory index-code`.
fn index(data_dir: &Path, repo_label: &str, repo_path: &Path) {
    comemory(
        data_dir,
        &[
            "index-code",
            "--repo",
            repo_label,
            "--path",
            repo_path.to_str().expect("utf8 path"),
        ],
    );
}

#[test]
fn ac3_two_indexed_repos_are_both_listed_fresh_with_nonzero_counts() {
    let home = TempDir::new().expect("home");
    let ws = TempDir::new().expect("workspace");
    let repo_a = build_repo(ws.path(), "repo-a", "a.rs", "fn alpha() {}\n");
    let repo_b = build_repo(ws.path(), "repo-b", "b.rs", "fn beta() {}\nfn gamma() {}\n");
    index(home.path(), "repo-a", &repo_a);
    index(home.path(), "repo-b", &repo_b);

    let repos = repos_json(home.path());
    assert_eq!(repos["repos"].as_array().expect("array").len(), 2);

    for (label, min_symbols) in [("repo-a", 1u64), ("repo-b", 2u64)] {
        let r = row(&repos, label);
        assert!(
            r["root_path"].as_str().is_some(),
            "{label} missing root_path"
        );
        assert!(
            r["files"].as_u64().expect("files") > 0,
            "{label} must have indexed at least one file"
        );
        assert!(
            r["symbols"].as_u64().expect("symbols") >= min_symbols,
            "{label} must have at least {min_symbols} symbols"
        );
        assert_eq!(r["status"], "fresh");
    }
}

#[test]
fn ac4_a_new_commit_without_reindexing_flips_only_that_repo_to_stale() {
    let home = TempDir::new().expect("home");
    let ws = TempDir::new().expect("workspace");
    let repo_a = build_repo(ws.path(), "repo-a", "a.rs", "fn alpha() {}\n");
    let repo_b = build_repo(ws.path(), "repo-b", "b.rs", "fn beta() {}\n");
    index(home.path(), "repo-a", &repo_a);
    index(home.path(), "repo-b", &repo_b);

    git_commit::commit_files(&repo_a, &[("extra.rs", "fn extra() {}\n")], "add extra");

    let repos = repos_json(home.path());
    assert_eq!(row(&repos, "repo-a")["status"], "stale");
    assert_eq!(
        row(&repos, "repo-b")["status"],
        "fresh",
        "an untouched repo must stay fresh"
    );
}

#[test]
fn ac4b_three_commits_report_three_changed_files_and_a_deleted_tree_is_unknown() {
    let home = TempDir::new().expect("home");
    let ws = TempDir::new().expect("workspace");
    let repo = build_repo(ws.path(), "repo-c", "a.rs", "fn alpha() {}\n");
    index(home.path(), "repo-c", &repo);

    git_commit::commit_files(&repo, &[("b.rs", "fn beta() {}\n")], "add b");
    git_commit::commit_files(&repo, &[("c.rs", "fn gamma() {}\n")], "add c");
    git_commit::commit_files(&repo, &[("d.rs", "fn delta() {}\n")], "add d");

    let repos = repos_json(home.path());
    let r = row(&repos, "repo-c");
    assert_eq!(r["status"], "stale");
    assert_eq!(r["changed_files"], serde_json::json!(3));

    std::fs::remove_dir_all(&repo).expect("delete working tree");
    let repos = repos_json(home.path());
    let r = row(&repos, "repo-c");
    assert_eq!(r["status"], "unknown");
    assert_eq!(r["changed_files"], serde_json::Value::Null);
}

#[test]
fn ac5_a_deleted_working_tree_reports_unknown_keeps_root_path_and_exits_0() {
    let home = TempDir::new().expect("home");
    let ws = TempDir::new().expect("workspace");
    let repo = build_repo(ws.path(), "repo-d", "a.rs", "fn alpha() {}\n");
    index(home.path(), "repo-d", &repo);

    let before = repos_json(home.path());
    let root_before = row(&before, "repo-d")["root_path"]
        .as_str()
        .expect("root_path before")
        .to_string();

    std::fs::remove_dir_all(&repo).expect("delete working tree");

    let status = Command::cargo_bin("comemory")
        .unwrap()
        .args(["repos", "--json"])
        .env("COMEMORY_DATA_DIR", home.path())
        .status()
        .expect("run comemory repos");
    assert!(status.success(), "comemory repos must exit 0");

    let after = repos_json(home.path());
    let r = row(&after, "repo-d");
    assert_eq!(r["status"], "unknown");
    assert_eq!(
        r["root_path"].as_str().expect("root_path after"),
        root_before,
        "root_path is reported unchanged even when the working tree is gone"
    );
}
