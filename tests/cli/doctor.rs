//! Task 15: `comemory doctor` reports SQLite-backed health: data dir,
//! db writability, schema version (currently `"2"`), and whether the
//! sqlite-vec extension loaded into the connection.
//!
//! The v0.1 report shape (`memories_count` / `index_failures`) is gone
//! in v0.2 — doctor now mirrors the v0.2 storage stack instead of the
//! markdown directory listing.

use assert_cmd::Command;
use tempfile::TempDir;

/// Build a `comemory` invocation with `COMEMORY_DATA_DIR` rooted at `home`.
fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

#[test]
fn doctor_reports_schema_version_two_on_fresh_dir() {
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
