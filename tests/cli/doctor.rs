//! Integration tests for `comemory doctor`.
//!
//! Covers schema_version "3", embed_hint round-trip via COMEMORY_EMBED_HINT,
//! and the v0.2 JSON report shape (data_dir, db_writable, sqlite_vec_loaded).

use assert_cmd::Command;
use tempfile::TempDir;

fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

#[test]
fn doctor_reports_schema_version_three_on_fresh_dir() {
    let home = TempDir::new().expect("tempdir");
    let assertion = bin(&home).arg("doctor").assert().success();
    let out = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(
        out.contains("schema_version") && out.contains(": 3"),
        "doctor should report schema_version 3 on a fresh dir: {out:?}"
    );
}

#[test]
fn doctor_json_emits_v2_report_shape() {
    let home = TempDir::new().expect("tempdir");
    let assertion = bin(&home).args(["--json", "doctor"]).assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert!(v["data_dir"].is_string());
    assert_eq!(v["db_writable"].as_bool(), Some(true));
    assert_eq!(v["schema_version"].as_str(), Some("3"));
    assert_eq!(v["sqlite_vec_loaded"].as_bool(), Some(true));
    // embed_hint must be present (null when not set).
    assert!(
        v.get("embed_hint").is_some(),
        "embed_hint field must exist in JSON"
    );
}

#[test]
fn doctor_schema_version_persists_after_save() {
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args(["save", "doctor save body", "--kind", "note"])
        .assert()
        .success();
    let assertion = bin(&home).args(["--json", "doctor"]).assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert_eq!(v["schema_version"].as_str(), Some("3"));
}

#[test]
fn doctor_embed_hint_round_trips_via_env_var() {
    let home = TempDir::new().expect("tempdir");
    let mut c = bin(&home);
    c.env("COMEMORY_EMBED_HINT", "ollama:nomic-embed-text");
    let assertion = c.args(["--json", "doctor"]).assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert_eq!(
        v["embed_hint"].as_str(),
        Some("ollama:nomic-embed-text"),
        "embed_hint must round-trip from env var; got: {v}"
    );

    // TTY output must also contain the hint.
    let mut c2 = bin(&home);
    c2.env("COMEMORY_EMBED_HINT", "ollama:nomic-embed-text");
    let tty = c2.arg("doctor").assert().success();
    let tty_out = String::from_utf8(tty.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(
        tty_out.contains("ollama:nomic-embed-text"),
        "TTY output must contain embed_hint; got: {tty_out:?}"
    );
}
