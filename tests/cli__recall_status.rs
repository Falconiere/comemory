#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Integration tests for `comemory recall-status`, driven as a real
//! subprocess: a saved memory plus a real tracked `find` (through the
//! binary, not a hand-forged `retrieval_log` row) must show up as one
//! pending query, then be judged by a real `feedback` call, and a `--since`
//! window in the future must report all zeros. Mirrors
//! `tests/cli__feedback.rs`'s subprocess harness.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

const REPO: &str = "app";

/// Build a `comemory` invocation rooted at `home`.
fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path());
    c
}

/// Run a `--json` subcommand to success and parse its stdout envelope.
fn run_json(home: &TempDir, args: &[&str]) -> Value {
    let mut cmd = bin(home);
    cmd.arg("--json").args(args);
    let assertion = cmd.assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    serde_json::from_str(stdout.trim()).expect("parse JSON envelope")
}

/// Save one real memory under [`REPO`], returning its id.
fn save(home: &TempDir, body: &str) -> String {
    let v = run_json(home, &["save", body, "--kind", "note", "--repo", REPO]);
    v["id"].as_str().expect("save id").to_string()
}

/// Run a real tracked `find` scoped to [`REPO`], returning its query id.
fn tracked_find(home: &TempDir, query: &str) -> String {
    let v = run_json(home, &["find", query, "--repo", REPO]);
    v["query_id"].as_str().expect("find query_id").to_string()
}

#[test]
fn a_tracked_recall_is_pending_until_judged() {
    let home = TempDir::new().expect("tempdir");
    let memory_id = save(&home, "recall status covers this memory");
    let query_id = tracked_find(&home, "recall status");

    let before = run_json(&home, &["recall-status", "--repo", REPO]);
    assert_eq!(before["repo"].as_str(), Some(REPO));
    assert_eq!(before["queries"].as_u64(), Some(1));
    assert_eq!(before["feedback_events"].as_u64(), Some(0));
    assert_eq!(before["saves"].as_u64(), Some(1));
    let pending = before["pending"].as_array().expect("pending array");
    assert_eq!(pending.len(), 1, "the tracked query has no verdict yet");
    assert_eq!(pending[0]["query_id"].as_str(), Some(query_id.as_str()));

    run_json(&home, &["feedback", &query_id, "--used", &memory_id]);

    let after = run_json(&home, &["recall-status", "--repo", REPO]);
    assert_eq!(after["feedback_events"].as_u64(), Some(1));
    assert!(
        after["pending"]
            .as_array()
            .expect("pending array")
            .is_empty(),
        "the query is judged, no longer pending: {after}"
    );
}

#[test]
fn a_since_in_the_future_reports_all_zeros_and_no_pending() {
    let home = TempDir::new().expect("tempdir");
    save(&home, "future window body");
    tracked_find(&home, "future window");

    let report = run_json(
        &home,
        &[
            "recall-status",
            "--repo",
            REPO,
            "--since",
            "2999-01-01T00:00:00Z",
        ],
    );
    assert_eq!(report["queries"].as_u64(), Some(0));
    assert_eq!(report["feedback_events"].as_u64(), Some(0));
    assert_eq!(report["saves"].as_u64(), Some(0));
    assert!(
        report["pending"]
            .as_array()
            .expect("pending array")
            .is_empty()
    );
}

#[test]
fn an_unparsable_since_is_a_usage_error() {
    let home = TempDir::new().expect("tempdir");
    let assertion = bin(&home)
        .args(["recall-status", "--since", "nonsense"])
        .assert()
        .failure();
    let output = assertion.get_output();
    assert_eq!(
        output.status.code(),
        Some(64),
        "an unparsable --since must exit EX_USAGE"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("since: invalid time value `nonsense`"),
        "stderr must name the flag and the offending value, got: {stderr}"
    );
}

#[test]
fn tty_output_prints_the_header_and_the_pending_line() {
    let home = TempDir::new().expect("tempdir");
    save(&home, "tty rendering coverage memory");
    let query_id = tracked_find(&home, "tty rendering");

    let out = bin(&home)
        .args(["recall-status", "--repo", REPO])
        .output()
        .expect("run comemory recall-status");
    assert!(
        out.status.success(),
        "recall-status failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    assert!(
        stdout.contains("queries=1  feedback_events=0  saves=1  pending=1"),
        "TTY header must report the exact counters: {stdout}"
    );
    assert!(
        stdout.contains(&query_id),
        "TTY output must name the pending query id: {stdout}"
    );
}
