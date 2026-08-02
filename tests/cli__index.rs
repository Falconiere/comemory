//! Test mirror for `src/cli/index.rs`, driven against the real binary
//! with `assert_cmd`. Real fixture files (never mocks) are copied into a
//! disposable temp source root per `common::docs_fixtures::seed`.

#[path = "common/docs_fixtures.rs"]
mod docs_fixtures;

use std::fs;

use assert_cmd::Command;
use serde::Deserialize;
use tempfile::tempdir;

/// Mirrors the JSON envelope `cli::index::run` emits.
#[derive(Deserialize)]
struct IndexOutput {
    sources: Vec<SourceReport>,
}

#[derive(Deserialize)]
struct SourceReport {
    source_id: String,
    indexed: usize,
    unchanged: usize,
    too_large: usize,
    unsupported: usize,
    errors: Vec<FileError>,
}

#[derive(Deserialize)]
struct FileError {
    kind: String,
}

fn run_index(home: &std::path::Path, args: &[&str]) -> assert_cmd::assert::Assert {
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home)
        .arg("index")
        .args(args)
        .assert()
}

fn parse_json(out: &[u8]) -> IndexOutput {
    serde_json::from_slice(out).expect("parse index --json output")
}

#[test]
fn index_registers_and_indexes_real_fixtures() {
    let home = tempdir().expect("home");
    let workspace = tempdir().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());

    let out = run_index(home.path(), &[docs.to_str().unwrap(), "--json"])
        .success()
        .get_output()
        .stdout
        .clone();
    let report = parse_json(&out);
    assert_eq!(report.sources.len(), 1);
    let s = &report.sources[0];
    assert_eq!(s.indexed, docs_fixtures::FIXTURE_COUNT);
    assert_eq!(s.too_large, 0);
    assert!(s.errors.is_empty());

    let conn = rusqlite::Connection::open(home.path().join("comemory.db")).expect("open db");
    let roots: i64 = conn
        .query_row("SELECT count(*) FROM source_roots", [], |r| r.get(0))
        .expect("count source_roots");
    assert_eq!(roots, 1);
    let docs_count: i64 = conn
        .query_row("SELECT count(*) FROM documents", [], |r| r.get(0))
        .expect("count documents");
    assert_eq!(docs_count as usize, docs_fixtures::FIXTURE_COUNT);
}

#[test]
fn reregistering_same_path_updates_not_duplicates() {
    let home = tempdir().expect("home");
    let workspace = tempdir().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());
    let path = docs.to_str().unwrap();

    run_index(home.path(), &[path, "--json"]).success();
    let out = run_index(home.path(), &[path, "--json"])
        .success()
        .get_output()
        .stdout
        .clone();
    let report = parse_json(&out);
    let s = &report.sources[0];
    // Unchanged fingerprints on the second run: nothing re-indexed.
    assert_eq!(s.indexed, 0);
    assert_eq!(s.unchanged, docs_fixtures::FIXTURE_COUNT);

    let conn = rusqlite::Connection::open(home.path().join("comemory.db")).expect("open db");
    let roots: i64 = conn
        .query_row("SELECT count(*) FROM source_roots", [], |r| r.get(0))
        .expect("count source_roots");
    assert_eq!(roots, 1, "re-registering must update, not duplicate");
    let files: i64 = conn
        .query_row("SELECT count(*) FROM source_files", [], |r| r.get(0))
        .expect("count source_files");
    assert_eq!(files as usize, docs_fixtures::FIXTURE_COUNT);
}

#[test]
fn overlap_rejection_message() {
    let home = tempdir().expect("home");
    let workspace = tempdir().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());

    run_index(home.path(), &[docs.to_str().unwrap()]).success();

    let child = docs.join("guide.md");
    let out = run_index(home.path(), &[child.to_str().unwrap()]).failure();
    let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("utf8 stderr");
    assert!(
        stderr.contains("overlaps"),
        "stderr must explain the overlap, got: {stderr}"
    );
}

#[test]
fn strict_exits_65_when_a_file_exceeds_max_file_bytes() {
    let home = tempdir().expect("home");
    let workspace = tempdir().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());

    // Real fixture sizes: changelog.txt 723B, data.csv 1111B,
    // page.html 1168B, guide.md 1790B — a 1000B ceiling keeps exactly
    // changelog.txt under it.
    let out = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .env("COMEMORY_INDEXING_MAX_FILE_BYTES", "1000")
        .args(["index", docs.to_str().unwrap(), "--strict", "--json"])
        .assert()
        .failure()
        .code(65)
        .get_output()
        .stdout
        .clone();
    let report = parse_json(&out);
    let s = &report.sources[0];
    assert_eq!(s.indexed, 1, "only changelog.txt fits under the ceiling");
    assert_eq!(s.too_large, 3);
    assert_eq!(s.errors.len(), 3);
    assert!(s.errors.iter().all(|e| e.kind == "too_large"));
}

/// Regression: a too_large fingerprint (size+mtime) is unchanged between
/// two runs against the same untouched files. Without the fingerprint's
/// `existing.status` guard, the second run's exact-match skip would
/// report `Unchanged` and silently drop the failure from `--strict`.
#[test]
fn strict_still_exits_65_on_a_second_run_against_the_same_too_large_files() {
    let home = tempdir().expect("home");
    let workspace = tempdir().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());
    let args = ["index", docs.to_str().unwrap(), "--strict", "--json"];

    run_with_ceiling(home.path(), &args).failure().code(65);

    let out = run_with_ceiling(home.path(), &args)
        .failure()
        .code(65)
        .get_output()
        .stdout
        .clone();
    let report = parse_json(&out);
    let s = &report.sources[0];
    assert_eq!(
        s.too_large, 3,
        "second run must still count all 3 too_large files, not report them Unchanged"
    );
    assert_eq!(s.errors.len(), 3);
}

fn run_with_ceiling(home: &std::path::Path, args: &[&str]) -> assert_cmd::assert::Assert {
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home)
        .env("COMEMORY_INDEXING_MAX_FILE_BYTES", "1000")
        .args(args)
        .assert()
}

#[test]
fn without_strict_the_same_partial_failure_exits_zero() {
    let home = tempdir().expect("home");
    let workspace = tempdir().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());

    let out = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .env("COMEMORY_INDEXING_MAX_FILE_BYTES", "1000")
        .args(["index", docs.to_str().unwrap(), "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let report = parse_json(&out);
    let s = &report.sources[0];
    assert_eq!(s.indexed, 1);
    assert_eq!(s.too_large, 3);
    // The source itself stays registered despite the per-file failures.
    assert!(!s.source_id.is_empty());
}

#[test]
fn strict_is_not_triggered_by_unsupported_files() {
    // A NUL byte under an allowlisted extension sniffs as binary and
    // classifies Unsupported (spec: "Everything else is recorded
    // `unsupported` for diagnostics") — not one of AC-7's three
    // strict-triggering outcomes (corrupt/unreadable/oversized), so
    // `--strict` must still exit 0. (A genuinely unreadable file cannot
    // be exercised through this end-to-end path: unix permissions gate
    // the discovery walk's own classification sniff identically to the
    // writer's later full read, so a chmod'd-000 file reclassifies
    // Unsupported before the writer ever runs — the writer's own
    // `Error(_)` outcome for a corrupt/unreadable file is covered
    // directly in `tests/document__writer.rs`.)
    let home = tempdir().expect("home");
    let workspace = tempdir().expect("workspace");
    let docs = workspace.path().join("mixed");
    fs::create_dir_all(&docs).expect("create dir");
    fs::write(docs.join("real.txt"), "hello world\n").expect("write real.txt");
    fs::write(docs.join("binary.txt"), b"\x00\x01\x02binary").expect("write binary.txt");

    let out = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["index", docs.to_str().unwrap(), "--strict", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let report = parse_json(&out);
    let s = &report.sources[0];
    assert_eq!(s.indexed, 1);
    assert_eq!(s.unsupported, 1);
    assert!(s.errors.is_empty());
}
