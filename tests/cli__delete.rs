#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
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

/// Save one memory through the real binary and return its id. `extra` is
/// appended after the body so callers can pass `--kind` or `--supersedes`;
/// with none, the CLI's own `--kind note` default applies.
fn save(home: &TempDir, body: &str, extra: &[&str]) -> String {
    let mut args = vec!["save", body];
    args.extend_from_slice(extra);
    let out = bin(home).args(&args).assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).expect("utf8");
    extract_saved_id(&stdout)
}

/// Read `memories.rank_score` for `id` from the DB under `data_dir`.
fn rank_score(data_dir: &std::path::Path, id: &str) -> f64 {
    let conn = connection::open(data_dir.join("comemory.db")).expect("open db");
    conn.query_row(
        "SELECT rank_score FROM memories WHERE id = ?1",
        rusqlite::params![id],
        |r| r.get(0),
    )
    .expect("rank_score row")
}

/// Seed the memory-rank fixture: one hub plus two replacements that
/// supersede it, so the hub owns every inlink in the memory graph. Returns
/// the hub id and the two survivor ids.
fn seed_hub_and_referrers(home: &TempDir) -> (String, [String; 2]) {
    let hub = save(home, "hub decision: pin the fts5 tokenizer version", &[]);
    let first = save(
        home,
        "replacement one: pin the tokenizer in the lockfile instead",
        &["--supersedes", &hub],
    );
    let second = save(
        home,
        "replacement two: vendor the tokenizer sources into the repo",
        &["--supersedes", &hub],
    );
    (hub, [first, second])
}

#[test]
fn delete_redistributes_memory_rank_to_survivors() {
    // AC-9. Deleting the hub drops it out of the node universe and the
    // delete trigger recomputes: the survivors — now edgeless, since their
    // outbound edges went with the hub — split the mass evenly, which is a
    // different score than either carried before.
    let home = TempDir::new().expect("tempdir");
    let data_dir = home.path().join(".comemory");
    let (hub, survivors) = seed_hub_and_referrers(&home);
    let before: Vec<f64> = survivors
        .iter()
        .map(|id| rank_score(&data_dir, id))
        .collect();
    let hub_rank = rank_score(&data_dir, &hub);
    assert!(
        hub_rank > before[0],
        "the hub holding both inlinks must lead: {hub_rank} vs {before:?}"
    );

    bin(&home).args(["delete", &hub]).assert().success();

    let after: Vec<f64> = survivors
        .iter()
        .map(|id| rank_score(&data_dir, id))
        .collect();
    for (id, (b, a)) in survivors.iter().zip(before.iter().zip(&after)) {
        assert!(
            (a - b).abs() > 1e-9,
            "{id} must be rescored when the hub leaves: {b} → {a}"
        );
    }
    assert!(
        (after[0] - after[1]).abs() < 1e-12,
        "two isolated survivors must share the mass evenly: {after:?}"
    );
}

#[test]
fn delete_stamps_deleted_at_in_sqlite() {
    let home = TempDir::new().expect("tempdir");
    let data_dir = home.path().join(".comemory");

    // Save a memory so comemory.db has a row.
    let id = save(&home, "delete stamps test body", &[]);

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

    let id = save(&home, "fts row removal test body", &[]);

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

    let id = save(
        &home,
        "edges removal test body",
        &["--kind", "decision", "--repo", "testrepo"],
    );

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

/// Memory ids returned by `comemory search <query> --json` under `home`.
fn search_ids(home: &TempDir, query: &str) -> Vec<String> {
    let assertion = bin(home)
        .args(["search", query, "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("json");
    v.get("hits")
        .and_then(serde_json::Value::as_array)
        .expect("hits array")
        .iter()
        .map(|h| h["memory_id"].as_str().expect("memory_id").to_string())
        .collect()
}

#[test]
fn deleted_memory_excluded_from_search() {
    let home = TempDir::new().expect("tempdir");
    let id = save(
        &home,
        "exclusive advisory lock decision for search exclusion",
        &["--kind", "decision"],
    );

    let before = search_ids(&home, "exclusive advisory lock");
    assert!(
        before.contains(&id),
        "memory must appear in search before delete; got: {before:?}"
    );

    bin(&home).args(["delete", &id]).assert().success();

    let after = search_ids(&home, "exclusive advisory lock");
    assert!(
        !after.contains(&id),
        "deleted memory must be excluded from search; got: {after:?}"
    );
}

#[test]
fn delete_missing_id_fails_without_enoent() {
    // Fresh data dir with no `memories/` yet: the missing-id case must
    // surface "memory not found", never a raw ENOENT from the data-dir
    // layout — whichever way the command chooses to create the layout first.
    let home = TempDir::new().expect("tempdir");
    let assertion = bin(&home)
        .args(["delete", "deadbeef0000"])
        .assert()
        .failure();
    let stderr = String::from_utf8(assertion.get_output().stderr.clone()).expect("utf8 stderr");
    assert!(
        stderr.contains("memory not found"),
        "stderr should mention 'memory not found', got: {stderr:?}"
    );
    assert!(
        !stderr.contains("No such file or directory"),
        "stderr should not surface raw ENOENT, got: {stderr:?}"
    );
}
