#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Integration tests for `comemory sync` (cloud push/pull CLI).

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

#[test]
fn sync_without_login_is_usage_error() {
    let home = TempDir::new().unwrap();
    bin(&home)
        .args(["sync", "--action", "status"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not logged in"));
}

#[test]
fn sync_action_help_lists_push_and_status() {
    let home = TempDir::new().unwrap();
    bin(&home)
        .args(["sync", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--action"))
        .stdout(predicate::str::contains("--workspace"))
        .stdout(predicate::str::contains("--allow-secret"));
}
