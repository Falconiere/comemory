//! Task 9: `comemory save` must write through the v0.2 store layer —
//! atomic markdown plus a SQLite mirror that includes FTS5 (always) and
//! `memory_vec` (only when a caller-supplied vector is provided).
//!
//! The dim guard fires before any DB write so a wrong-dim vector is a
//! hard failure on stderr instead of a silently dropped row. The fixup
//! commit also reorders the guard ahead of the markdown write, so a
//! wrong-dim payload must leave the `memories/` directory empty.

use assert_cmd::Command;
use comemory::store::connection;
use std::fs;
use tempfile::tempdir;

use super::vectors;

/// Count `.md` files at the top of `<data_dir>/memories/`, ignoring the
/// `.trash/` subdir and any hidden tmp files. Returns 0 when the directory
/// does not exist yet (the wrong-dim path is allowed to skip `ensure_dirs`
/// in the future without breaking this assertion).
fn count_md_files(data_dir: &std::path::Path) -> usize {
    let mem_dir = data_dir.join("memories");
    let Ok(read) = fs::read_dir(&mem_dir) else {
        return 0;
    };
    read.flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|n| n.ends_with(".md") && !n.starts_with('.'))
                .unwrap_or(false)
        })
        .count()
}

/// Run a single `count(*)` query against `conn` and return the integer
/// result. Tests can chain several without a forest of `query_row` boilerplate.
fn count_query(conn: &rusqlite::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0))
        .unwrap_or_else(|e| panic!("query failed: {sql}: {e}"))
}

/// Assert the SQLite mirror tables for the
/// `save_writes_md_and_indexes_lexical_when_no_vector` test: one row in
/// `memories` (with the expected repo+author), exactly two tag rows,
/// `memory_fts` populated, `memory_vec` empty, and the 4 expected
/// memory→{repo,author,tag} edges.
fn assert_lexical_save_mirror(conn: &rusqlite::Connection) {
    assert_eq!(count_query(conn, "SELECT count(*) FROM memory_fts"), 1);
    assert_eq!(count_query(conn, "SELECT count(*) FROM memory_vec"), 0);
    assert_eq!(
        count_query(
            conn,
            "SELECT count(*) FROM memories WHERE repo = 'foo' AND author = 'alice'",
        ),
        1,
        "memories row missing or with wrong repo/author",
    );
    assert_eq!(count_query(conn, "SELECT count(*) FROM memory_tags"), 2);
    // 1 in_repo + 1 authored_by + 2 tagged = 4 edges.
    assert_eq!(
        count_query(
            conn,
            "SELECT count(*) FROM edges WHERE src_kind = 'memory' \
              AND rel IN ('in_repo','authored_by','tagged')",
        ),
        4,
        "expected 1 in_repo + 1 authored_by + 2 tagged",
    );
}

#[test]
fn save_writes_md_and_indexes_lexical_when_no_vector() {
    let home = tempdir().expect("tempdir");
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args([
            "save",
            "--kind",
            "note",
            "--repo",
            "foo",
            "--tags",
            "db,postgres",
            "--author",
            "alice",
            "advisory locks for migration ordering",
        ])
        .assert()
        .success();

    assert_eq!(count_md_files(home.path()), 1);

    let db_path = home.path().join("comemory.db");
    let conn = connection::open(&db_path).expect("open db");
    assert_lexical_save_mirror(&conn);
}

#[test]
fn save_with_vector_stdin_writes_memory_vec_row() {
    let home = tempdir().expect("tempdir");
    let vector = vectors::vector("seed", 1024);
    let payload = serde_json::to_string(&serde_json::json!({ "embedding": vector })).expect("json");

    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args([
            "save",
            "--vector-stdin",
            "--kind",
            "note",
            "advisory locks for migration ordering",
        ])
        .write_stdin(payload)
        .assert()
        .success();

    let db_path = home.path().join("comemory.db");
    let conn = connection::open(&db_path).expect("open db");
    let vec_count: i64 = conn
        .query_row("SELECT count(*) FROM memory_vec", [], |r| r.get(0))
        .expect("count vec");
    assert_eq!(vec_count, 1);
}

#[test]
fn save_with_vector_csv_flag_writes_memory_vec_row() {
    // Exercise the CSV (`--vector`) branch of `read_optional_vector` end-
    // to-end: comma-split, f32::parse, then dim-guard. A 1024-d vector
    // rendered as CSV is rejected by some shells for being too long, but
    // `assert_cmd` passes the string straight through to `argv` so this
    // works fine in-process.
    let home = tempdir().expect("tempdir");
    let vector = vectors::vector("csv-seed", 1024);
    let csv = vector
        .iter()
        .map(|f| f.to_string())
        .collect::<Vec<_>>()
        .join(",");

    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args([
            "save",
            "--vector",
            &csv,
            "--kind",
            "note",
            "advisory locks for migration ordering via csv",
        ])
        .assert()
        .success();

    let db_path = home.path().join("comemory.db");
    let conn = connection::open(&db_path).expect("open db");
    let vec_count: i64 = conn
        .query_row("SELECT count(*) FROM memory_vec", [], |r| r.get(0))
        .expect("count vec");
    assert_eq!(vec_count, 1, "CSV --vector flag should populate memory_vec");
}

#[test]
fn save_csv_vector_with_bad_token_fails_to_parse() {
    // The CSV branch maps `f32::parse` errors into `Error::Config("--vector
    // parse: ..")`. Feed a token that isn't a float to exercise that path.
    let home = tempdir().expect("tempdir");
    let assertion = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args([
            "save",
            "--vector",
            "0.1,not-a-float,0.3",
            "--kind",
            "note",
            "body",
        ])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(
        stderr.contains("--vector parse"),
        "stderr should mention CSV parse error, got: {stderr}",
    );

    // Parse failure happens before the markdown write, so nothing on disk.
    assert_eq!(
        count_md_files(home.path()),
        0,
        "csv parse error must not leave an orphan markdown",
    );
}

#[test]
fn save_rejects_wrong_dim_vector() {
    let home = tempdir().expect("tempdir");
    let bad = vectors::vector("seed", 16);
    let payload = serde_json::to_string(&serde_json::json!({ "embedding": bad })).expect("json");

    let assertion = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["save", "--vector-stdin", "--kind", "note", "body"])
        .write_stdin(payload)
        .assert()
        .failure();
    let out = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(out.contains("vector dim mismatch"), "stderr: {out}");

    // Major-finding fix: the dim guard now runs BEFORE `MemoryStore::save`,
    // so a wrong-dim payload must leave the `memories/` directory empty —
    // no orphan markdown to garbage-collect.
    assert_eq!(
        count_md_files(home.path()),
        0,
        "dim guard must abort before the markdown write — orphan .md detected",
    );
}
