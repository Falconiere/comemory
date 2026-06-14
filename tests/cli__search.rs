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
fn search_json_emits_query_id_backed_by_retrieval_log_row() {
    // The envelope's query_id must be a valid `q-<yyyymmdd>-<8hex>` id and
    // must round-trip to a `retrieval_log` row in the same db, so
    // `comemory feedback <id>` can attribute the session later.
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
    let qid = v
        .get("query_id")
        .and_then(Value::as_str)
        .expect("query_id in envelope")
        .to_string();
    assert!(
        comemory::stats::feedback::is_valid_query_id(&qid),
        "query_id shape, got: {qid}"
    );

    let db = rusqlite::Connection::open_with_flags(
        home.path().join("comemory.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open db read-only");
    let (query, returned): (String, String) = db
        .query_row(
            "SELECT query, returned_ids FROM retrieval_log WHERE query_id = ?1",
            [&qid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("retrieval_log row for emitted query_id");
    assert_eq!(query, "advisory lock");
    let ids: Vec<String> = serde_json::from_str(&returned).expect("returned_ids json");
    let hits = v.get("hits").and_then(Value::as_array).expect("hits array");
    assert_eq!(ids.len(), hits.len(), "logged ids must match emitted hits");
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
