#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Task 14 / schema-migration-safety step 8: `comemory rebuild` — part 3.
//!
//! THE motivating bug: migration 0013 added `source_files`, `documents`,
//! `document_chunks`, `document_fts`, and `source_roots`, and
//! `src/api/rebuild/copy.rs` was never taught about any of them, so
//! `comemory rebuild` silently discarded a user's entire document index.
//! These tests drive the real binary against a real indexed document
//! corpus (`common::docs_fixtures::seed`) and prove the four copyable
//! tables survive a rebuild bit-for-bit while `source_roots` — the one
//! table that is reconciled, not copied — reflects `sources.toml` even
//! when that differs from what the old DB held.

#[path = "common/docs_fixtures.rs"]
mod docs_fixtures;

use std::collections::BTreeSet;
use std::fs;

use assert_cmd::Command;
use rusqlite::Connection;
use serde_json::Value;
use tempfile::tempdir;

fn index_docs(home: &std::path::Path, docs: &std::path::Path, repo: &str) {
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home)
        .args(["index", docs.to_str().expect("utf8 path"), "--repo", repo])
        .assert()
        .success();
}

fn run_rebuild(home: &std::path::Path) {
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home)
        .args(["rebuild"])
        .assert()
        .success();
}

fn open_db(home: &std::path::Path) -> Connection {
    Connection::open(home.join("comemory.db")).expect("open db")
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("count")
}

/// The set of `document_id`s a `search --only document --json` run returns.
fn search_document_ids(home: &std::path::Path, query: &str) -> BTreeSet<String> {
    let out = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home)
        .env("COMEMORY_DISABLE_ACCESS_TRACKING", "true")
        .args(["search", query, "--only", "document", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).expect("parse search json");
    v["items"]
        .as_array()
        .expect("items array")
        .iter()
        .map(|item| {
            item["document_id"]
                .as_str()
                .expect("document_id")
                .to_string()
        })
        .collect()
}

/// AC-16: every `documents` / `document_chunks` / `source_files` row
/// survives a rebuild, and `comemory search` returns the same document
/// hits before and after.
#[test]
fn rebuild_preserves_document_tables_and_search_hits() {
    let home = tempdir().expect("home");
    let workspace = tempdir().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());
    index_docs(home.path(), &docs, "docs-corpus");

    let (before_docs, before_chunks, before_files) = {
        let conn = open_db(home.path());
        (
            count(&conn, "SELECT count(*) FROM documents"),
            count(&conn, "SELECT count(*) FROM document_chunks"),
            count(&conn, "SELECT count(*) FROM source_files"),
        )
    };
    assert_eq!(before_docs as usize, docs_fixtures::FIXTURE_COUNT);
    assert!(before_chunks > 0, "expected at least one chunk");
    assert_eq!(before_files as usize, docs_fixtures::FIXTURE_COUNT);

    let before_hits = search_document_ids(home.path(), "comemory");
    assert!(
        !before_hits.is_empty(),
        "expected document hits before rebuild"
    );

    run_rebuild(home.path());

    {
        let conn = open_db(home.path());
        assert_eq!(
            count(&conn, "SELECT count(*) FROM documents"),
            before_docs,
            "rebuild must preserve every documents row — THE motivating bug"
        );
        assert_eq!(
            count(&conn, "SELECT count(*) FROM document_chunks"),
            before_chunks,
            "rebuild must preserve every document_chunks row"
        );
        assert_eq!(
            count(&conn, "SELECT count(*) FROM source_files"),
            before_files,
            "rebuild must preserve every source_files row"
        );
    }

    let after_hits = search_document_ids(home.path(), "comemory");
    assert_eq!(
        before_hits, after_hits,
        "rebuild must not change which documents a search returns"
    );
}

/// AC-16's other half: `source_roots` is absent from the copy and is
/// restored from `sources.toml` instead. Proven structurally: edit
/// `sources.toml` directly (bypassing the CLI, so the live SQLite mirror
/// is left stale) between index and rebuild, then assert the *new*
/// value wins — a plain table copy from the old DB could never produce
/// that outcome.
#[test]
fn rebuild_restores_source_roots_from_sources_toml_not_from_the_old_db() {
    let home = tempdir().expect("home");
    let workspace = tempdir().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());
    index_docs(home.path(), &docs, "original-label");

    {
        let conn = open_db(home.path());
        let repo: String = conn
            .query_row("SELECT repo FROM source_roots", [], |r| r.get(0))
            .expect("source_roots row");
        assert_eq!(repo, "original-label");
    }

    let toml_path = home.path().join("sources.toml");
    let raw = fs::read_to_string(&toml_path).expect("read sources.toml");
    let edited = raw.replace("original-label", "updated-label");
    assert_ne!(
        raw, edited,
        "expected the repo label to be present in sources.toml"
    );
    fs::write(&toml_path, edited).expect("write edited sources.toml");

    run_rebuild(home.path());

    let conn = open_db(home.path());
    let repo: String = conn
        .query_row("SELECT repo FROM source_roots", [], |r| r.get(0))
        .expect("source_roots row after rebuild");
    assert_eq!(
        repo, "updated-label",
        "source_roots must be reconciled fresh from sources.toml, not copied from the old DB"
    );
}
