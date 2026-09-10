#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Integration tests for `comemory distill`.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

fn fixture() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/claude-code-session-saves.jsonl")
}

#[test]
fn distill_help_lists_required_flags() {
    let home = TempDir::new().unwrap();
    bin(&home)
        .args(["distill", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--session-id"))
        .stdout(predicate::str::contains("--transcript"))
        .stdout(predicate::str::contains("--dry-run"))
        .stdout(predicate::str::contains("--api-url"));
}

#[test]
fn distill_without_login_is_usage_error() {
    let home = TempDir::new().unwrap();
    bin(&home)
        .args([
            "distill",
            "--session-id",
            "sess-1",
            "--transcript",
            fixture().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not logged in"));
}

#[test]
fn distill_dry_run_json_recovers_six_real_saves() {
    let home = TempDir::new().unwrap();
    let assert = bin(&home)
        .args([
            "distill",
            "--session-id",
            "sess-fixture",
            "--transcript",
            fixture().to_str().unwrap(),
            "--dry-run",
            "--json",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("json");
    assert_eq!(v["dryRun"], true);
    assert_eq!(v["extracted"], 6);
    assert_eq!(v["batch"]["extractor"], "claude-code-explicit-save");
    assert_eq!(v["batch"]["candidates"].as_array().unwrap().len(), 6);
    assert_eq!(
        v["batch"]["candidates"][0]["title"],
        "comemory.io feature gap map (2026-09-10)"
    );
    assert_eq!(v["batch"]["candidates"][5]["kind"], "pattern");
}
