//! Integration test for the rewired `comemory search` subcommand.
//!
//! Asserts that after saving a memory through the CLI the lexical FTS path
//! returns at least one hit when `--json` is requested. No embedder is
//! invoked because no `--vector` / `--vector-stdin` is supplied: the router
//! goes straight to the lexical branch.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::tempdir;

#[test]
fn search_finds_seeded_memory_lexically() {
    let home = tempdir().expect("tempdir");
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args([
            "save",
            "--kind",
            "decision",
            "--repo",
            "foo",
            "postgres advisory locks for migration ordering",
        ])
        .assert()
        .success();

    let assert = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["search", "advisory lock", "--json"])
        .assert()
        .success();
    let out = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    let v: Value = serde_json::from_str(&out).expect("json");
    let hits = v.get("hits").and_then(Value::as_array).expect("hits array");
    assert!(!hits.is_empty(), "got: {out}");
}

#[test]
fn kind_filter_limits_hits_to_matching_kind() {
    // `--kind decision` must drop the bug memory even though both bodies
    // match the query lexically. The saved decision id (from `save --json`)
    // pins the surviving hit.
    let home = tempdir().expect("tempdir");
    let save = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args([
            "save",
            "--kind",
            "decision",
            "--json",
            "postgres advisory locks chosen for migration ordering",
        ])
        .assert()
        .success();
    let save_out = String::from_utf8_lossy(&save.get_output().stdout).to_string();
    let decision_id = serde_json::from_str::<Value>(&save_out)
        .expect("save json")
        .get("id")
        .and_then(Value::as_str)
        .expect("save id")
        .to_string();
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args([
            "save",
            "--kind",
            "bug",
            "postgres pool exhaustion observed under load",
        ])
        .assert()
        .success();

    let assert = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["search", "postgres", "--kind", "decision", "--json"])
        .assert()
        .success();
    let out = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    let v: Value = serde_json::from_str(&out).expect("json");
    let hits = v.get("hits").and_then(Value::as_array).expect("hits array");
    assert_eq!(hits.len(), 1, "only the decision memory may survive: {out}");
    assert_eq!(
        hits[0].get("memory_id").and_then(Value::as_str),
        Some(decision_id.as_str()),
        "surviving hit must be the decision memory: {out}"
    );

    // Without the filter both memories match.
    let assert = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["search", "postgres", "--json"])
        .assert()
        .success();
    let out = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    let v: Value = serde_json::from_str(&out).expect("json");
    let hits = v.get("hits").and_then(Value::as_array).expect("hits array");
    assert_eq!(hits.len(), 2, "unfiltered search must keep both: {out}");
}

#[test]
fn identifier_query_finds_prose_only_memory_in_top_3() {
    // Spec promise: searching the identifier `VecDimMismatch` must surface
    // a memory whose body describes the "dim mismatch" in prose without
    // ever containing the identifier verbatim. Exercises the router's
    // subtoken OR tier end-to-end through the real binary.
    let home = tempdir().expect("tempdir");
    let body = "embedder returned wrong dim mismatch against the vec table";
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["save", "--kind", "bug", body])
        .assert()
        .success();

    let assert = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["search", "VecDimMismatch", "--k", "3", "--json"])
        .assert()
        .success();
    let out = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    let v: Value = serde_json::from_str(&out).expect("json");
    let hits = v.get("hits").and_then(Value::as_array).expect("hits array");
    assert!(
        !hits.is_empty(),
        "identifier query must reach the prose-only body, got: {out}"
    );
}
