//! Shared fixture for the `comemory architecture` suites: a real two-directory
//! git repository, really indexed by the real `comemory index-code`, so every
//! architecture assertion runs against mined edges and indexed paths rather
//! than a hand-built database.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use tempfile::TempDir;

#[path = "git_commit.rs"]
mod git_commit;
#[path = "git_repo.rs"]
mod git_repo;

/// The `comemory` binary bound to `home`'s data dir.
pub fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

/// Create and index a two-directory repo under `label`: `src/a/one.rs`
/// imports `src/b/two.rs`, and both are committed together twice so the
/// co-change miner records the pair.
pub fn index_repo(home: &TempDir, workspace: &Path, label: &str) -> PathBuf {
    let repo = workspace.join(label);
    git_repo::init_repo(&repo);
    git_commit::commit_files(
        &repo,
        &[
            ("src/a/one.rs", "use crate::b::two;\n\npub fn alpha() {}\n"),
            ("src/b/two.rs", "pub fn beta() {}\n"),
        ],
        "couple once",
    );
    git_commit::commit_files(
        &repo,
        &[
            (
                "src/a/one.rs",
                "use crate::b::two;\n\npub fn alpha() { let _x = 1; }\n",
            ),
            ("src/b/two.rs", "pub fn beta() { let _y = 2; }\n"),
        ],
        "couple twice",
    );
    reindex(home, &repo, label);
    repo
}

/// Re-run the real indexer over `repo` after the working tree changed.
pub fn reindex(home: &TempDir, repo: &Path, label: &str) {
    bin(home)
        .args(["index-code", "--repo", label, "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();
}

/// Re-index from scratch. Incremental indexing only visits files that still
/// exist, so a deleted file keeps its `indexed_files` row until a full run —
/// which is what makes a stale member observable.
pub fn reindex_full(home: &TempDir, repo: &Path, label: &str) {
    bin(home)
        .args(["index-code", "--repo", label, "--mode", "full", "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();
}

/// Run `comemory architecture <args…>` and return parsed JSON stdout.
pub fn architecture_json(home: &TempDir, args: &[&str]) -> serde_json::Value {
    let out = stdout_of(home, args);
    serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("invalid json: {e}\n{out}"))
}

/// Run `comemory architecture <args…>` and return raw stdout.
pub fn stdout_of(home: &TempDir, args: &[&str]) -> String {
    let mut full = vec!["architecture"];
    full.extend_from_slice(args);
    let out = bin(home).args(&full).assert().success();
    String::from_utf8(out.get_output().stdout.clone()).expect("utf8")
}

/// Scaffold `label` and save the result, returning the saved model JSON.
pub fn scaffold_and_save(home: &TempDir, label: &str, dir: &Path) -> serde_json::Value {
    let model = architecture_json(home, &["scaffold", "--repo", label, "--json"]);
    let path = dir.join("model.json");
    std::fs::write(&path, serde_json::to_string_pretty(&model).expect("json")).expect("write");
    bin(home)
        .args(["architecture", "save"])
        .arg(&path)
        .args(["--repo", label])
        .assert()
        .success();
    model
}

/// Commit `files` into `repo` — the fixture's own commit helper, so a suite
/// that changes the tree does not have to include `git_commit` a second time.
pub fn commit_files(repo: &Path, files: &[(&str, &str)], message: &str) {
    git_commit::commit_files(repo, files, message);
}
