//! Integration test for the rewired `comemory context` subcommand.
//!
//! Saves a single memory then asserts `comemory context --json` returns a
//! bundle whose `memories` array contains that memory. The lexical router
//! path is exercised so no embedder is required.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::tempdir;

#[test]
fn context_returns_bundle_for_seeded_memory() {
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
        .args(["context", "advisory lock", "--json"])
        .assert()
        .success();
    let out = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    let v: Value = serde_json::from_str(&out).expect("json");
    assert_eq!(
        v.get("query").and_then(Value::as_str),
        Some("advisory lock")
    );
    let memories = v
        .get("memories")
        .and_then(Value::as_array)
        .expect("memories array");
    assert!(!memories.is_empty(), "got: {out}");
}
