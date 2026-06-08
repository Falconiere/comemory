//! Task 9: `comemory save` must write through the v0.2 store layer —
//! atomic markdown plus a SQLite mirror that includes FTS5 (always) and
//! `memory_vec` (only when a caller-supplied vector is provided).
//!
//! The dim guard fires before any DB write so a wrong-dim vector is a
//! hard failure on stderr instead of a silently dropped row.

use assert_cmd::Command;
use comemory::store::connection;
use std::fs;
use tempfile::tempdir;

use super::vectors;

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
            "advisory locks for migration ordering",
        ])
        .assert()
        .success();

    let mem_dir = home.path().join("memories");
    let files: Vec<_> = fs::read_dir(&mem_dir)
        .expect("read")
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|n| n.ends_with(".md"))
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(files.len(), 1);

    let db_path = home.path().join("comemory.db");
    let conn = connection::open(&db_path).expect("open db");
    let count: i64 = conn
        .query_row("SELECT count(*) FROM memory_fts", [], |r| r.get(0))
        .expect("count fts");
    assert_eq!(count, 1);
    let vec_count: i64 = conn
        .query_row("SELECT count(*) FROM memory_vec", [], |r| r.get(0))
        .expect("count vec");
    assert_eq!(vec_count, 0);
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
}
