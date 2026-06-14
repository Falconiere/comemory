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

/// Seed `n` lexically-matching memories through the real CLI.
fn seed_many(home: &tempfile::TempDir, n: usize) {
    for i in 0..n {
        Command::cargo_bin("comemory")
            .expect("bin")
            .env("COMEMORY_DATA_DIR", home.path())
            .args([
                "save",
                "--kind",
                "note",
                &format!("paging corpus row {i} about sqlite indexing"),
            ])
            .assert()
            .success();
    }
}

/// Run `comemory search --json` with extra args, return the parsed envelope.
fn search_json(home: &tempfile::TempDir, extra: &[&str]) -> Value {
    let mut args = vec!["search", "sqlite indexing", "--json"];
    args.extend_from_slice(extra);
    let assert = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(&args)
        .assert()
        .success();
    let out = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("json ({e}): {out}"))
}

fn ids_of(v: &Value) -> Vec<String> {
    v.get("hits")
        .and_then(Value::as_array)
        .expect("hits array")
        .iter()
        .map(|h| h["memory_id"].as_str().expect("memory_id").to_string())
        .collect()
}

/// CLI-level stability: page through 25 matches with `--k 8` and rising
/// `--offset`. No id repeats across pages, none is skipped, and the
/// concatenation equals the single-shot ranked window. The envelope's
/// `limit`/`offset`/`has_more`/`total` track each page correctly.
#[test]
fn search_pagination_is_stable_across_offsets() {
    let home = tempdir().expect("tempdir");
    seed_many(&home, 25);

    // Single-shot ground truth: the whole window.
    let full = search_json(&home, &["--k", "0"]);
    let full_ids = ids_of(&full);
    let total = full["total"].as_u64().expect("total") as usize;
    assert_eq!(full_ids.len(), total, "total == in-window ranked count");
    // The diversified window may collapse a near-dup or two; it must still
    // hold more than two pages so the stability walk is meaningful.
    assert!(total > 16, "need > 2 pages of distinct hits: {total}");
    assert_eq!(
        full["has_more"],
        Value::Bool(false),
        "whole window: no more"
    );

    let page_size = 8;
    let mut seen = std::collections::HashSet::new();
    let mut joined: Vec<String> = Vec::new();
    let mut offset = 0;
    loop {
        let v = search_json(&home, &["--k", "8", "--offset", &offset.to_string()]);
        assert_eq!(v["limit"].as_u64(), Some(8), "limit echoes --k");
        assert_eq!(v["offset"].as_u64(), Some(offset as u64), "offset echoed");
        assert_eq!(v["total"].as_u64(), Some(total as u64), "stable total");
        let ids = ids_of(&v);
        for id in &ids {
            assert!(seen.insert(id.clone()), "id {id} on two pages (overlap)");
        }
        joined.extend(ids);
        let expect_more = offset + page_size < full_ids.len();
        assert_eq!(
            v["has_more"],
            Value::Bool(expect_more),
            "has_more wrong at offset {offset}"
        );
        if !expect_more {
            break;
        }
        offset += page_size;
    }
    assert_eq!(
        joined, full_ids,
        "concatenated pages must reproduce the single-shot ranked window"
    );
}

#[test]
fn search_limit_is_a_visible_alias_of_k() {
    let home = tempdir().expect("tempdir");
    seed_many(&home, 10);
    let via_k = ids_of(&search_json(&home, &["--k", "3"]));
    let via_limit = ids_of(&search_json(&home, &["--limit", "3"]));
    assert_eq!(via_k.len(), 3, "k bounds the page");
    assert_eq!(via_k, via_limit, "--limit must alias --k exactly");
}

#[test]
fn search_offset_beyond_window_is_empty_with_no_more() {
    let home = tempdir().expect("tempdir");
    seed_many(&home, 5);
    let v = search_json(&home, &["--k", "5", "--offset", "9999"]);
    assert!(ids_of(&v).is_empty(), "offset past the window is empty");
    assert_eq!(v["has_more"], Value::Bool(false), "nothing beyond");
    assert!(v["total"].as_u64().unwrap() >= 5, "total still reported");
}
