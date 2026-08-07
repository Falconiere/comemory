#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/cli/unindex.rs`.

#[path = "common/docs_fixtures.rs"]
mod docs_fixtures;

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use comemory::store::connection;
use serde::Deserialize;
use tempfile::tempdir;

#[derive(Deserialize)]
struct IndexOutput {
    sources: Vec<IndexedSource>,
}

#[derive(Deserialize)]
struct IndexedSource {
    source_id: String,
}

#[derive(Deserialize)]
struct UnindexOutput {
    source_id: String,
    documents_removed: usize,
}

/// Index `docs` under `home`, optionally under `repo`, and return the
/// registered source id.
fn index_and_get_source_id(home: &Path, docs: &Path, repo: Option<&str>) -> String {
    let mut args = vec!["index", docs.to_str().unwrap(), "--json"];
    if let Some(repo) = repo {
        args.push("--repo");
        args.push(repo);
    }
    let out = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home)
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let report: IndexOutput = serde_json::from_slice(&out).expect("parse index --json");
    report.sources[0].source_id.clone()
}

/// Count `edges` rows carrying relation `rel`.
fn edge_count(conn: &rusqlite::Connection, rel: &str) -> i64 {
    conn.query_row("SELECT count(*) FROM edges WHERE rel = ?1", [rel], |r| {
        r.get(0)
    })
    .unwrap_or_else(|_| panic!("count edges rel={rel}"))
}

/// Read every fixture file's raw bytes, keyed by file name, for a
/// before/after AC-6 comparison.
fn snapshot(docs: &Path) -> Vec<(String, Vec<u8>)> {
    let mut entries: Vec<(String, Vec<u8>)> = fs::read_dir(docs)
        .expect("read docs dir")
        .map(|e| e.expect("dir entry"))
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let bytes = fs::read(e.path()).expect("read fixture file");
            (name, bytes)
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

fn empty_db_counts(conn: &rusqlite::Connection) -> (i64, i64, i64, i64, i64) {
    // Table names can't be bound as parameters, so each is a complete
    // literal SQL string rather than format!-spliced (same spirit as
    // the store/sources.rs const-SQL fix).
    let count = |sql: &str| -> i64 {
        conn.query_row(sql, [], |r| r.get(0))
            .unwrap_or_else(|_| panic!("count via: {sql}"))
    };
    (
        count("SELECT count(*) FROM source_roots"),
        count("SELECT count(*) FROM source_files"),
        count("SELECT count(*) FROM documents"),
        count("SELECT count(*) FROM document_chunks"),
        count("SELECT count(*) FROM document_fts"),
    )
}

#[test]
fn unindex_by_source_id_removes_derived_rows_but_leaves_files_untouched() {
    let home = tempdir().expect("home");
    let workspace = tempdir().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());
    let before = snapshot(&docs);

    let source_id = index_and_get_source_id(home.path(), &docs, None);

    let out = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["unindex", &source_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let report: UnindexOutput = serde_json::from_slice(&out).expect("parse unindex --json");
    assert_eq!(report.source_id, source_id);
    assert_eq!(report.documents_removed, docs_fixtures::FIXTURE_COUNT);

    let conn = connection::open(home.path().join("comemory.db")).expect("open db");
    assert_eq!(empty_db_counts(&conn), (0, 0, 0, 0, 0));

    let after = snapshot(&docs);
    assert_eq!(
        before, after,
        "unindex must never modify source files (AC-6)"
    );
}

#[test]
fn unindex_by_path_resolves_the_registered_canonical_path() {
    let home = tempdir().expect("home");
    let workspace = tempdir().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());

    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["index", docs.to_str().unwrap()])
        .assert()
        .success();

    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["unindex", docs.to_str().unwrap()])
        .assert()
        .success();

    let conn = connection::open(home.path().join("comemory.db")).expect("open db");
    assert_eq!(empty_db_counts(&conn), (0, 0, 0, 0, 0));
}

#[test]
fn unindex_purges_member_of_source_and_references_document_edges() {
    let home = tempdir().expect("home");
    let workspace = tempdir().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());

    let source_id = index_and_get_source_id(home.path(), &docs, Some("demo"));

    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["save", "see `demo:guide.md` for setup", "--repo", "demo"])
        .assert()
        .success();

    {
        let conn = connection::open(home.path().join("comemory.db")).expect("open db");
        assert!(
            edge_count(&conn, "member_of_source") > 0,
            "indexing must derive member_of_source edges"
        );
        assert_eq!(
            edge_count(&conn, "references_document"),
            1,
            "save must resolve the backtick reference to the indexed document"
        );
    }

    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["unindex", &source_id])
        .assert()
        .success();

    let conn = connection::open(home.path().join("comemory.db")).expect("open db");
    assert_eq!(
        edge_count(&conn, "member_of_source"),
        0,
        "unindex must purge member_of_source edges"
    );
    assert_eq!(
        edge_count(&conn, "references_document"),
        0,
        "unindex must purge references_document edges"
    );
}

#[test]
fn unindex_unknown_target_is_not_found() {
    let home = tempdir().expect("home");
    let out = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["unindex", "0123456789abcdef0123456789abcdef"])
        .assert()
        .failure()
        .code(64)
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8(out).expect("utf8 stderr");
    assert!(stderr.contains("no registered source matches"));
}
