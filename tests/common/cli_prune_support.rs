//! Shared helpers for `tests/cli__prune.rs` and `tests/cli__prune_2.rs`.

use assert_cmd::Command;
use tempfile::TempDir;

/// Build a `comemory` invocation with `COMEMORY_DATA_DIR` rooted at `home`.
pub fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

/// Save a memory via the real binary and return its id from the JSON output.
pub fn save_memory(home: &TempDir, body: &str) -> String {
    let assertion = bin(home)
        .args(["--json", "save", body, "--kind", "note"])
        .assert()
        .success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse save JSON");
    v["id"].as_str().expect("save emits id").to_string()
}

/// Make the memory prune-eligible by doctoring the mirror row: drop the
/// quality to 2 and back-date `last_accessed` so the activation falls below
/// the default −2.0 floor. Saves carry no feedback row, so the Beta posterior
/// sits exactly at the 0.25 ceiling (inclusive).
pub fn make_prune_eligible(home: &TempDir, id: &str) {
    let db = home.path().join(".comemory").join("comemory.db");
    let conn = comemory::store::connection::open(db).expect("open mirror");
    conn.execute(
        "UPDATE memories SET quality = 2, last_accessed = '2025-01-01T00:00:00Z' WHERE id = ?1",
        [id],
    )
    .expect("doctor row");
}
