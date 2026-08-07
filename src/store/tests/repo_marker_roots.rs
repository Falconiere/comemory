#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/store/repo_marker_roots.rs`.

use comemory::store::{code_row, connection, repo_marker_roots};
use rusqlite::Connection;
use tempfile::TempDir;

fn open_db() -> (Connection, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let conn = connection::open(tmp.path().join("comemory.db")).expect("open db");
    (conn, tmp)
}

#[test]
fn all_roots_is_empty_with_no_repo_marker_rows() {
    let (conn, _tmp) = open_db();
    assert_eq!(
        repo_marker_roots::all_roots(&conn).expect("all_roots"),
        Vec::<std::path::PathBuf>::new()
    );
}

#[test]
fn all_roots_returns_distinct_canonicalized_paths_and_skips_null() {
    let (conn, _tmp) = open_db();
    let repo_a = TempDir::new().expect("repo a");
    let repo_b = TempDir::new().expect("repo b");
    let root_a = repo_a.path().canonicalize().expect("canonicalize a");
    let root_b = repo_b.path().canonicalize().expect("canonicalize b");

    code_row::upsert_repo_root(&conn, "repo-a", &root_a.to_string_lossy()).expect("upsert a");
    code_row::upsert_repo_root(&conn, "repo-b", &root_b.to_string_lossy()).expect("upsert b");
    // A NULL `root_path` marker row (a repo indexed before v7, no --root
    // stored) must be skipped, not errored on.
    conn.execute(
        "INSERT INTO repo_marker(repo, root_path) VALUES('repo-c', NULL)",
        [],
    )
    .expect("insert null-root marker");

    let mut roots = repo_marker_roots::all_roots(&conn).expect("all_roots");
    roots.sort();
    let mut expected = vec![root_a, root_b];
    expected.sort();
    assert_eq!(roots, expected);
}

#[test]
fn all_roots_skips_a_row_whose_directory_no_longer_exists() {
    let (conn, _tmp) = open_db();
    let live = TempDir::new().expect("live repo");
    let root_live = live.path().canonicalize().expect("canonicalize live");
    code_row::upsert_repo_root(&conn, "live", &root_live.to_string_lossy()).expect("upsert live");

    // A stored root pointing at a directory deleted since indexing.
    let gone = TempDir::new().expect("gone repo");
    let root_gone = gone.path().canonicalize().expect("canonicalize gone");
    code_row::upsert_repo_root(&conn, "gone", &root_gone.to_string_lossy()).expect("upsert gone");
    drop(gone);
    std::fs::remove_dir_all(&root_gone).ok();

    let roots = repo_marker_roots::all_roots(&conn).expect("all_roots");
    assert_eq!(roots, vec![root_live]);
}
