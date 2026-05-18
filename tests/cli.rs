//! Integration tests for `qwick` memory-only subcommands.
//!
//! Each test isolates state by pointing `QWICK_DATA_DIR` at a fresh
//! `tempfile::TempDir`. We cover the side-effecting flow (save -> list ->
//! delete) and the diagnostic command (doctor).

use assert_cmd::Command;
use tempfile::TempDir;

/// Build a `qwick` invocation with `QWICK_DATA_DIR` rooted at `home`.
fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("qwick").expect("cargo_bin qwick");
    c.env("QWICK_DATA_DIR", home.path().join(".qwick"));
    c
}

/// Extract the id from `saved <id>` in the save command's stdout.
fn extract_saved_id(stdout: &str) -> String {
    let line = stdout
        .lines()
        .find(|l| l.starts_with("saved "))
        .expect("save stdout has 'saved <id>' line");
    line.strip_prefix("saved ")
        .expect("strip 'saved ' prefix")
        .split_whitespace()
        .next()
        .expect("id token after 'saved '")
        .to_string()
}

#[test]
fn save_then_list_shows_id() {
    let home = TempDir::new().expect("tempdir");
    let save = bin(&home)
        .args(["save", "hello world from save_then_list", "--kind", "note"])
        .assert()
        .success();
    let saved_stdout = String::from_utf8(save.get_output().stdout.clone()).expect("utf8 stdout");
    let id = extract_saved_id(&saved_stdout);
    assert_eq!(id.len(), 8, "memory id should be 8 hex chars: {id:?}");

    let list = bin(&home).args(["list"]).assert().success();
    let out = String::from_utf8(list.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(
        out.contains(&id),
        "list output should mention saved id {id}: {out:?}"
    );
    // The list row format is `<id>  <kind>  <repo>  <slug>`; confirm the kind
    // we passed lands in the row for the saved id.
    let row = out
        .lines()
        .find(|l| l.contains(&id))
        .expect("row for saved id");
    assert!(
        row.contains("note"),
        "row should include kind 'note': {row:?}"
    );
}

#[test]
fn doctor_reports_zero_memories_on_fresh_dir() {
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicates::str::contains("memories_count : 0"));
}

#[test]
fn doctor_count_grows_after_save() {
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args(["save", "first memory body", "--kind", "decision"])
        .assert()
        .success();
    bin(&home)
        .args(["save", "second memory body", "--kind", "bug"])
        .assert()
        .success();
    let doctor = bin(&home).arg("doctor").assert().success();
    let out = String::from_utf8(doctor.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(
        out.contains("memories_count : 2"),
        "doctor should report 2 memories, got: {out:?}"
    );
}

#[test]
fn save_then_delete_removes_from_list() {
    let home = TempDir::new().expect("tempdir");
    let save = bin(&home)
        .args(["save", "delete-me memory body", "--kind", "note"])
        .assert()
        .success();
    let saved_stdout = String::from_utf8(save.get_output().stdout.clone()).expect("utf8 stdout");
    let id = extract_saved_id(&saved_stdout);

    bin(&home)
        .args(["delete", &id])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!("deleted {id}")));

    let list = bin(&home).args(["list"]).assert().success();
    let out = String::from_utf8(list.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(
        !out.contains(&id),
        "deleted id should not appear in list: {out:?}"
    );
}

#[test]
fn save_json_emits_id_and_path() {
    let home = TempDir::new().expect("tempdir");
    let save = bin(&home)
        .args([
            "--json",
            "save",
            "json mode save body",
            "--kind",
            "discovery",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(save.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    let id = v["id"].as_str().expect("id field is a string").to_string();
    let path = v["path"].as_str().expect("path field is a string");
    assert_eq!(id.len(), 8, "id should be 8 hex chars");
    assert!(
        path.ends_with(".md"),
        "path should point at a .md file: {path}"
    );
}

#[test]
fn list_json_emits_array() {
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args(["save", "first list-json body", "--kind", "note"])
        .assert()
        .success();
    bin(&home)
        .args([
            "save",
            "second list-json body",
            "--kind",
            "bug",
            "--repo",
            "alpha",
        ])
        .assert()
        .success();
    let list = bin(&home).args(["--json", "list"]).assert().success();
    let stdout = String::from_utf8(list.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    let arr = v.as_array().expect("top-level JSON array");
    assert_eq!(arr.len(), 2, "two rows expected, got {arr:?}");
    for row in arr {
        assert!(row["id"].is_string());
        assert!(row["kind"].is_string());
        assert!(row["repo"].is_string());
        assert!(row["slug"].is_string());
    }
}

#[test]
fn list_filters_by_repo_and_kind() {
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args([
            "save",
            "alpha decision body",
            "--kind",
            "decision",
            "--repo",
            "alpha",
        ])
        .assert()
        .success();
    bin(&home)
        .args(["save", "beta bug body", "--kind", "bug", "--repo", "beta"])
        .assert()
        .success();
    bin(&home)
        .args(["save", "alpha bug body", "--kind", "bug", "--repo", "alpha"])
        .assert()
        .success();

    let filtered = bin(&home)
        .args(["list", "--repo", "alpha", "--kind", "bug"])
        .assert()
        .success();
    let out = String::from_utf8(filtered.get_output().stdout.clone()).expect("utf8 stdout");
    let line_count = out.lines().filter(|l| !l.is_empty()).count();
    assert_eq!(line_count, 1, "exactly one alpha+bug row expected: {out:?}");
    assert!(out.contains("alpha"));
    assert!(out.contains("bug"));
}

#[test]
fn feedback_records_used_and_irrelevant_ids() {
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args(["feedback", "q1", "--used", "aaa,bbb", "--irrelevant", "ccc"])
        .assert()
        .success()
        .stdout(predicates::str::contains("ok"));
}

#[test]
fn delete_missing_id_fails() {
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args(["delete", "deadbeef0000"])
        .assert()
        .failure();
}

#[test]
fn doctor_json_emits_object() {
    let home = TempDir::new().expect("tempdir");
    let doctor = bin(&home).args(["--json", "doctor"]).assert().success();
    let stdout = String::from_utf8(doctor.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert!(v["data_dir"].is_string());
    assert_eq!(v["memories_count"].as_u64(), Some(0));
}
