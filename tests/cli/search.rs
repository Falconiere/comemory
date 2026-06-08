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
