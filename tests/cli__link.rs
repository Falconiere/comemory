#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Integration tests for `comemory link` (allowlist cache override).

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

#[test]
fn link_requires_repo() {
    let home = TempDir::new().unwrap();
    bin(&home)
        .args(["link", "--workspace", "ws_test"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--repo"));
}

#[test]
fn link_writes_sync_repos_override() {
    let home = TempDir::new().unwrap();
    bin(&home)
        .args([
            "--json",
            "link",
            "--workspace",
            "ws_org",
            "--repo",
            "codasignal/foo",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("codasignal/foo"))
        .stdout(predicate::str::contains("ws_org"));
    let cfg = std::fs::read_to_string(home.path().join(".comemory/config.toml")).unwrap();
    assert!(cfg.contains("[sync.repos]") || cfg.contains("codasignal/foo"));
}
