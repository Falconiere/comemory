#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/store/repo_marker.rs` — the `last_mined_commit`
//! cursor read/advance pair.

use comemory::store::connection;
use comemory::store::repo_marker::{
    advance_mined_cursor, all_repos, archived, exists, last_mined_commit, read_for_lazy_reindex,
    root_path, set_archived,
};
use rusqlite::Connection;
use tempfile::TempDir;

fn open_db() -> (Connection, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let conn = connection::open(tmp.path().join("comemory.db")).expect("open db");
    (conn, tmp)
}

#[test]
fn last_mined_commit_is_none_with_no_repo_marker_row() {
    let (conn, _tmp) = open_db();
    assert_eq!(
        last_mined_commit(&conn, "no-such-repo").expect("read"),
        None
    );
}

#[test]
fn advance_then_read_round_trips() {
    let (conn, _tmp) = open_db();
    advance_mined_cursor(&conn, "r", "abc123").expect("advance");
    assert_eq!(
        last_mined_commit(&conn, "r").expect("read"),
        Some("abc123".to_string())
    );

    // A second advance overwrites rather than accumulates.
    advance_mined_cursor(&conn, "r", "def456").expect("advance again");
    assert_eq!(
        last_mined_commit(&conn, "r").expect("read"),
        Some("def456".to_string())
    );
}

#[test]
fn advance_preserves_other_repo_marker_columns() {
    let (conn, _tmp) = open_db();
    conn.execute(
        "INSERT INTO repo_marker(repo, root_path) VALUES('r', '/some/root')",
        [],
    )
    .expect("seed root_path");

    advance_mined_cursor(&conn, "r", "abc123").expect("advance");

    let root_path: Option<String> = conn
        .query_row(
            "SELECT root_path FROM repo_marker WHERE repo = 'r'",
            [],
            |row| row.get(0),
        )
        .expect("read root_path");
    assert_eq!(root_path, Some("/some/root".to_string()));
}

#[test]
fn read_for_lazy_reindex_is_none_with_no_marker_row() {
    let (conn, _tmp) = open_db();
    assert!(
        read_for_lazy_reindex(&conn, "no-such-repo")
            .expect("read")
            .is_none()
    );
}

#[test]
fn read_for_lazy_reindex_returns_all_three_columns() {
    let (conn, _tmp) = open_db();
    conn.execute(
        "INSERT INTO repo_marker(repo, last_mined_commit, root_path, archived) \
         VALUES ('r', 'abc123', '/some/root', 1)",
        [],
    )
    .expect("seed marker");

    let marker = read_for_lazy_reindex(&conn, "r")
        .expect("read")
        .expect("row present");
    assert_eq!(marker.last_mined_commit, Some("abc123".to_string()));
    assert_eq!(marker.root_path, Some("/some/root".to_string()));
    assert!(marker.archived);
}

#[test]
fn archived_is_none_with_no_marker_row() {
    let (conn, _tmp) = open_db();
    assert_eq!(archived(&conn, "no-such-repo").expect("read"), None);
}

#[test]
fn archived_reports_the_stored_flag() {
    let (conn, _tmp) = open_db();
    conn.execute(
        "INSERT INTO repo_marker(repo, archived) VALUES ('r', 1)",
        [],
    )
    .expect("seed archived marker");
    conn.execute(
        "INSERT INTO repo_marker(repo, archived) VALUES ('other', 0)",
        [],
    )
    .expect("seed unarchived marker");

    assert_eq!(archived(&conn, "r").expect("read"), Some(true));
    assert_eq!(archived(&conn, "other").expect("read"), Some(false));
}

#[test]
fn all_repos_lists_every_label_ascending() {
    let (conn, _tmp) = open_db();
    conn.execute("INSERT INTO repo_marker(repo) VALUES('zeta')", [])
        .expect("seed zeta");
    conn.execute("INSERT INTO repo_marker(repo) VALUES('alpha')", [])
        .expect("seed alpha");

    assert_eq!(
        all_repos(&conn).expect("all_repos"),
        vec!["alpha".to_string(), "zeta".to_string()]
    );
}

#[test]
fn root_path_is_none_with_no_marker_row_or_a_null_root() {
    let (conn, _tmp) = open_db();
    assert_eq!(root_path(&conn, "no-such-repo").expect("read"), None);

    conn.execute("INSERT INTO repo_marker(repo) VALUES('r')", [])
        .expect("seed marker with no root");
    assert_eq!(root_path(&conn, "r").expect("read"), None);
}

#[test]
fn root_path_reports_the_stored_root() {
    let (conn, _tmp) = open_db();
    conn.execute(
        "INSERT INTO repo_marker(repo, root_path) VALUES('r', '/some/root')",
        [],
    )
    .expect("seed root_path");
    assert_eq!(
        root_path(&conn, "r").expect("read"),
        Some("/some/root".to_string())
    );
}

#[test]
fn exists_reflects_marker_row_presence() {
    let (conn, _tmp) = open_db();
    assert!(!exists(&conn, "r").expect("exists"));
    conn.execute("INSERT INTO repo_marker(repo) VALUES('r')", [])
        .expect("seed marker");
    assert!(exists(&conn, "r").expect("exists"));
}

#[test]
fn set_archived_flips_the_flag_and_reports_zero_rows_for_an_unknown_repo() {
    let (conn, _tmp) = open_db();
    conn.execute("INSERT INTO repo_marker(repo) VALUES('r')", [])
        .expect("seed marker");

    let updated = set_archived(&conn, "r", true).expect("set archived");
    assert_eq!(updated, 1);
    assert_eq!(archived(&conn, "r").expect("read"), Some(true));

    let updated = set_archived(&conn, "r", false).expect("set unarchived");
    assert_eq!(updated, 1);
    assert_eq!(archived(&conn, "r").expect("read"), Some(false));

    let unknown = set_archived(&conn, "ghost", true).expect("set on unknown repo");
    assert_eq!(unknown, 0);
}
