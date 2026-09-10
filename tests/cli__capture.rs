#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Integration tests for `comemory capture` (CLI-only).

#[path = "common/capture_platform_server.rs"]
mod capture_platform_server;

use std::fs;
use std::io::Write as _;

use assert_cmd::Command;
use capture_platform_server::{CapturePlatformServer, CapturePlatformState};
use predicates::prelude::*;
use tempfile::TempDir;

fn fixture() -> String {
    format!(
        "{}/tests/common/fixtures/claude-code-session.jsonl",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

fn write_auth(home: &TempDir, api_url: &str) {
    let dir = home.path().join(".comemory");
    fs::create_dir_all(&dir).unwrap();
    let auth = serde_json::json!({
        "version": 2,
        "secret": "cmk_testsecret",
        "key_prefix": "cmk_test",
        "api_url": api_url,
        "organization_id": "org",
        "organization_slug": "org",
        "organization_name": "Org",
        "workspace_id": "ws-org"
    });
    fs::write(
        dir.join("auth.json"),
        serde_json::to_vec_pretty(&auth).unwrap(),
    )
    .unwrap();
}

#[test]
fn session_dry_run_against_fixture() {
    let home = TempDir::new().unwrap();
    write_auth(&home, "http://127.0.0.1:9"); // unused on dry-run
    let out = bin(&home)
        .args([
            "--json",
            "capture",
            "session",
            "--path",
            &fixture(),
            "--dry-run",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(
        v.get("dry_run").and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        v.get("posted").and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        v.pointer("/receipt/redaction/version")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        v.pointer("/receipt/turnCount")
            .and_then(serde_json::Value::as_u64),
        Some(24)
    );
}

#[test]
fn session_posts_to_loopback() {
    let server = CapturePlatformServer::spawn(CapturePlatformState::default());
    let home = TempDir::new().unwrap();
    write_auth(&home, &server.base_url);
    bin(&home)
        .args(["--json", "capture", "session", "--path", &fixture()])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"posted\":true"));
    let reqs = server.requests();
    let post = reqs
        .iter()
        .find(|r| r.method == "POST" && r.path == "/v1/sessions")
        .expect("POST /v1/sessions");
    assert_eq!(post.workspace_header.as_deref(), Some("ws-org"));
    assert!(post.authorization.starts_with("Bearer "));
}

#[test]
fn sources_lists_consent() {
    let server = CapturePlatformServer::spawn(CapturePlatformState::default());
    let home = TempDir::new().unwrap();
    write_auth(&home, &server.base_url);
    bin(&home)
        .args(["--json", "capture", "sources"])
        .assert()
        .success()
        .stdout(predicate::str::contains("claude-code"));
}

#[test]
fn install_hook_writes_session_end() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("settings.json");
    bin(&home)
        .args([
            "capture",
            "install-hook",
            "--settings",
            settings.to_str().unwrap(),
        ])
        .assert()
        .success();
    let raw = fs::read_to_string(&settings).unwrap();
    assert!(raw.contains("SessionEnd"));
    assert!(raw.contains("comemory-capture-session-end"));
}

#[test]
fn cursor_source_is_usage_error() {
    let home = TempDir::new().unwrap();
    write_auth(&home, "http://127.0.0.1:9");
    bin(&home)
        .args([
            "capture",
            "session",
            "--source",
            "cursor",
            "--path",
            &fixture(),
            "--dry-run",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not implemented"));
}

#[test]
fn from_hook_reads_stdin() {
    let home = TempDir::new().unwrap();
    write_auth(&home, "http://127.0.0.1:9");
    let payload = serde_json::json!({
        "session_id": "ignored-when-path-set",
        "transcript_path": fixture(),
    });
    bin(&home)
        .args(["--json", "capture", "session", "--from-hook", "--dry-run"])
        .write_stdin(payload.to_string())
        .assert()
        .success()
        .stdout(predicate::str::contains("\"dry_run\":true"));
}

#[test]
fn install_hook_force_merges() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("settings.json");
    let mut f = fs::File::create(&settings).unwrap();
    writeln!(
        f,
        r#"{{"hooks":{{"SessionEnd":[{{"hooks":[{{"type":"command","command":"echo foreign"}}]}}]}}}}"#
    )
    .unwrap();
    bin(&home)
        .args([
            "capture",
            "install-hook",
            "--settings",
            settings.to_str().unwrap(),
            "--force",
        ])
        .assert()
        .success();
    let raw = fs::read_to_string(&settings).unwrap();
    assert!(raw.contains("echo foreign"));
    assert!(raw.contains("comemory-capture-session-end"));
}
