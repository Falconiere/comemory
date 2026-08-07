#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/store/sources.rs`.

use comemory::store::connection;
use comemory::store::sources::{self, SourceRootUpsert};
use rusqlite::Connection;
use tempfile::TempDir;

fn open_db() -> (Connection, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let conn = connection::open(tmp.path().join("comemory.db")).expect("open db");
    (conn, tmp)
}

fn seed(conn: &Connection, id: &str, path: &str, repo: Option<&str>) {
    sources::upsert(
        conn,
        SourceRootUpsert {
            id,
            canonical_path: path,
            kind: "dir",
            repo,
            created_at: "2026-01-01T00:00:00.000000000Z",
            updated_at: "2026-01-01T00:00:00.000000000Z",
        },
    )
    .expect("seed upsert");
}

#[test]
fn upsert_then_get_round_trips_columns() {
    let (conn, _tmp) = open_db();
    seed(&conn, "aaaa", "/repo/docs", Some("myrepo"));

    let row = sources::get(&conn, "aaaa")
        .expect("get")
        .expect("row exists");
    assert_eq!(row.canonical_path, "/repo/docs");
    assert_eq!(row.kind, "dir");
    assert_eq!(row.repo.as_deref(), Some("myrepo"));
    assert_eq!(row.status, "active", "fresh row defaults to active");
    assert_eq!(row.created_at, "2026-01-01T00:00:00.000000000Z");
}

#[test]
fn get_missing_id_returns_none() {
    let (conn, _tmp) = open_db();
    assert_eq!(sources::get(&conn, "nope").expect("get"), None);
}

#[test]
fn upsert_on_conflict_refreshes_fields_but_preserves_status_and_created_at() {
    let (conn, _tmp) = open_db();
    seed(&conn, "bbbb", "/repo/docs", None);
    conn.execute(
        "UPDATE source_roots SET status = 'unreachable' WHERE id = 'bbbb'",
        [],
    )
    .expect("mark unreachable");

    sources::upsert(
        &conn,
        SourceRootUpsert {
            id: "bbbb",
            canonical_path: "/repo/docs-renamed",
            kind: "file",
            repo: Some("new-repo"),
            created_at: "2099-01-01T00:00:00.000000000Z",
            updated_at: "2026-06-01T00:00:00.000000000Z",
        },
    )
    .expect("second upsert");

    let row = sources::get(&conn, "bbbb")
        .expect("get")
        .expect("row exists");
    assert_eq!(row.canonical_path, "/repo/docs-renamed");
    assert_eq!(row.kind, "file");
    assert_eq!(row.repo.as_deref(), Some("new-repo"));
    assert_eq!(row.updated_at, "2026-06-01T00:00:00.000000000Z");
    assert_eq!(row.status, "unreachable", "status must survive the upsert");
    assert_eq!(
        row.created_at, "2026-01-01T00:00:00.000000000Z",
        "created_at must survive the upsert"
    );
}

#[test]
fn delete_removes_the_row() {
    let (conn, _tmp) = open_db();
    seed(&conn, "cccc", "/repo/x", None);
    sources::delete(&conn, "cccc").expect("delete");
    assert_eq!(sources::get(&conn, "cccc").expect("get"), None);
}

#[test]
fn list_is_ordered_by_canonical_path() {
    let (conn, _tmp) = open_db();
    seed(&conn, "id-z", "/z/path", None);
    seed(&conn, "id-a", "/a/path", None);

    let rows = sources::list(&conn).expect("list");
    let paths: Vec<&str> = rows.iter().map(|r| r.canonical_path.as_str()).collect();
    assert_eq!(paths, vec!["/a/path", "/z/path"]);
}

#[test]
fn count_reflects_row_count() {
    let (conn, _tmp) = open_db();
    assert_eq!(sources::count(&conn).expect("count"), 0);
    seed(&conn, "id-1", "/a", None);
    seed(&conn, "id-2", "/b", None);
    assert_eq!(sources::count(&conn).expect("count"), 2);
}
