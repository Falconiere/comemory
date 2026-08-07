#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/cli/sources.rs`.

#[path = "common/docs_fixtures.rs"]
mod docs_fixtures;

use std::path::Path;

use assert_cmd::Command;
use serde::Deserialize;
use tempfile::tempdir;

#[derive(Deserialize)]
struct Row {
    canonical_path: String,
    kind: String,
    repo: Option<String>,
    status: String,
    indexed: usize,
    error: usize,
    stale: usize,
}

#[test]
fn sources_is_empty_before_any_registration() {
    let home = tempdir().expect("home");
    let out = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["sources", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let rows: Vec<Row> = serde_json::from_slice(&out).expect("parse sources --json");
    assert!(rows.is_empty());

    // TTY mode prints an explicit empty-state line rather than nothing.
    let tty_out = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .arg("sources")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(tty_out).expect("utf8 stdout");
    assert!(text.contains("no sources registered"));
}

#[test]
fn sources_lists_registered_source_with_counts() {
    let home = tempdir().expect("home");
    let workspace = tempdir().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());

    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["index", docs.to_str().unwrap(), "--repo", "docs-corpus"])
        .assert()
        .success();

    let out = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["sources", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let rows: Vec<Row> = serde_json::from_slice(&out).expect("parse sources --json");
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(r.kind, "dir");
    assert_eq!(r.repo.as_deref(), Some("docs-corpus"));
    assert_eq!(r.status, "active");
    assert_eq!(r.indexed, docs_fixtures::FIXTURE_COUNT);
    assert_eq!(r.error, 0);
    assert_eq!(r.stale, 0);
    assert!(
        Path::new(&r.canonical_path).ends_with("docs"),
        "canonical_path's final component must be exactly `docs`, got: {}",
        r.canonical_path
    );
}
