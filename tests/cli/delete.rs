//! Integration tests for `comemory delete`.
//!
//! Verifies that a soft-deleted memory is excluded from `search`, `list`,
//! and that the SQLite mirror (`memories.deleted_at`, edges, `memory_fts`)
//! is updated atomically.

use assert_cmd::Command;
use comemory::store::connection;
use tempfile::TempDir;

/// Build a `comemory` invocation with `COMEMORY_DATA_DIR` rooted at `home`.
fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

/// Extract the 8-hex id from the `saved <id>` line in save stdout.
fn extract_saved_id(stdout: &str) -> String {
    stdout
        .lines()
        .find(|l| l.starts_with("saved "))
        .expect("save stdout has 'saved <id>' line")
        .strip_prefix("saved ")
        .expect("strip prefix")
        .split_whitespace()
        .next()
        .expect("id token")
        .to_string()
}

#[test]
fn delete_stamps_deleted_at_in_sqlite() {
    let home = TempDir::new().expect("tempdir");
    let data_dir = home.path().join(".comemory");

    // Save a memory so comemory.db has a row.
    let save_out = bin(&home)
        .args(["save", "delete stamps test body", "--kind", "note"])
        .assert()
        .success();
    let stdout = String::from_utf8(save_out.get_output().stdout.clone()).expect("utf8");
    let id = extract_saved_id(&stdout);

    // Verify deleted_at is NULL before delete.
    let conn = connection::open(data_dir.join("comemory.db")).expect("open db");
    let deleted_at_before: Option<String> = conn
        .query_row(
            "SELECT deleted_at FROM memories WHERE id = ?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .expect("row exists");
    assert!(
        deleted_at_before.is_none(),
        "deleted_at must be NULL before delete"
    );

    // Soft-delete.
    bin(&home)
        .args(["delete", &id])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!("deleted {id}")));

    // deleted_at must now be set.
    let deleted_at_after: Option<String> = conn
        .query_row(
            "SELECT deleted_at FROM memories WHERE id = ?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .expect("row still present");
    assert!(
        deleted_at_after.is_some(),
        "deleted_at must be set after delete"
    );
}

#[test]
fn delete_removes_fts_row() {
    let home = TempDir::new().expect("tempdir");
    let data_dir = home.path().join(".comemory");

    let save_out = bin(&home)
        .args(["save", "fts row removal test body", "--kind", "note"])
        .assert()
        .success();
    let stdout = String::from_utf8(save_out.get_output().stdout.clone()).expect("utf8");
    let id = extract_saved_id(&stdout);

    bin(&home).args(["delete", &id]).assert().success();

    let conn = connection::open(data_dir.join("comemory.db")).expect("open db");
    let fts_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memory_fts WHERE memory_id = ?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(fts_count, 0, "memory_fts row must be removed after delete");
}

#[test]
fn delete_removes_edges() {
    let home = TempDir::new().expect("tempdir");
    let data_dir = home.path().join(".comemory");

    let save_out = bin(&home)
        .args([
            "save",
            "edges removal test body",
            "--kind",
            "decision",
            "--repo",
            "testrepo",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(save_out.get_output().stdout.clone()).expect("utf8");
    let id = extract_saved_id(&stdout);

    // Verify at least one edge was created (in_repo).
    let conn = connection::open(data_dir.join("comemory.db")).expect("open db");
    let edge_count_before: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE src_kind = 'memory' AND src_id = ?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .expect("count edges before");
    assert!(edge_count_before > 0, "edges must exist before delete");

    bin(&home).args(["delete", &id]).assert().success();

    let edge_count_after: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE src_kind = 'memory' AND src_id = ?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .expect("count edges after");
    assert_eq!(
        edge_count_after, 0,
        "all touching edges must be removed after delete"
    );
}

#[test]
fn deleted_memory_excluded_from_search() {
    let home = TempDir::new().expect("tempdir");

    let save_out = bin(&home)
        .args([
            "save",
            "exclusive advisory lock decision for search exclusion",
            "--kind",
            "decision",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(save_out.get_output().stdout.clone()).expect("utf8");
    let id = extract_saved_id(&stdout);

    // Verify the memory shows up in search before deletion.
    let search_before = bin(&home)
        .args(["search", "exclusive advisory lock", "--json"])
        .assert()
        .success();
    let out_before = String::from_utf8(search_before.get_output().stdout.clone()).expect("utf8");
    let v: serde_json::Value = serde_json::from_str(out_before.trim()).expect("json");
    let hits_before = v.as_array().expect("array");
    assert!(
        hits_before
            .iter()
            .any(|h| h["memory_id"].as_str() == Some(&id)),
        "memory must appear in search before delete; got: {out_before}"
    );

    // Soft-delete.
    bin(&home).args(["delete", &id]).assert().success();

    // Memory must no longer appear in search.
    let search_after = bin(&home)
        .args(["search", "exclusive advisory lock", "--json"])
        .assert()
        .success();
    let out_after = String::from_utf8(search_after.get_output().stdout.clone()).expect("utf8");
    let v2: serde_json::Value = serde_json::from_str(out_after.trim()).expect("json");
    let hits_after = v2.as_array().expect("array");
    assert!(
        !hits_after
            .iter()
            .any(|h| h["memory_id"].as_str() == Some(&id)),
        "deleted memory must be excluded from search; got: {out_after}"
    );
}
