//! Test mirror for `src/store/code_row.rs`.
//!
//! Verifies that the shared `code_symbols` insert helper returns the
//! freshly-assigned PK and writes every supplied column verbatim.

use comemory::store::code_row::{self, CodeSymbolRow};
use comemory::store::connection;
use tempfile::TempDir;

/// Insert one whole-symbol `code_symbols` row via the production writer and
/// return its rowid; lines fixed at `(1, 10)`. Shared by the access-bump
/// tests below.
fn seed(conn: &rusqlite::Connection, symbol: &str) -> i64 {
    code_row::insert(
        conn,
        &CodeSymbolRow {
            repo: "r",
            path: "src/lib.rs",
            blob_oid: "oid",
            symbol,
            kind: "function",
            lang: "rust",
            line_start: 1,
            line_end: 10,
            snippet: "fn body() {}",
            simhash: 0,
            parent_id: None,
        },
    )
    .expect("insert code symbol")
}

/// Read `(access_count, last_accessed)` for a `code_symbols` rowid.
fn access_row(conn: &rusqlite::Connection, id: i64) -> (i64, Option<String>) {
    conn.query_row(
        "SELECT access_count, last_accessed FROM code_symbols WHERE id = ?1",
        rusqlite::params![id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .expect("access row")
}

#[test]
fn record_access_bumps_only_listed_ids_and_stamps_last_accessed() {
    let tmp = TempDir::new().expect("tempdir");
    let conn = connection::open(tmp.path().join("comemory.db")).expect("open db");

    let hit = seed(&conn, "hit_fn");
    let untouched = seed(&conn, "untouched_fn");

    // Fresh rows: zero accesses, last_accessed defaulted to indexed_at.
    let (hit_before, _) = access_row(&conn, hit);
    let (untouched_before, untouched_stamp_before) = access_row(&conn, untouched);
    assert_eq!(hit_before, 0, "fresh row starts at zero accesses");
    assert_eq!(untouched_before, 0);

    code_row::record_access(&conn, &[hit]);

    let (hit_after, hit_stamp) = access_row(&conn, hit);
    assert_eq!(hit_after, 1, "listed id must be bumped exactly once");
    assert!(
        hit_stamp.is_some(),
        "listed id must carry a last_accessed timestamp"
    );

    let (untouched_after, untouched_stamp_after) = access_row(&conn, untouched);
    assert_eq!(
        untouched_after, 0,
        "an id absent from the list must not be bumped"
    );
    assert_eq!(
        untouched_stamp_after, untouched_stamp_before,
        "an absent id's last_accessed must be left untouched"
    );

    // A second bump accumulates.
    code_row::record_access(&conn, &[hit]);
    assert_eq!(access_row(&conn, hit).0, 2, "bumps accumulate");
}

#[test]
fn record_access_with_empty_ids_is_a_no_op() {
    // The `track`-off gate suppresses the call by handing the writer no
    // ids; that must touch nothing (no UPDATE runs at all).
    let tmp = TempDir::new().expect("tempdir");
    let conn = connection::open(tmp.path().join("comemory.db")).expect("open db");

    let id = seed(&conn, "lone_fn");
    let before = access_row(&conn, id);

    code_row::record_access(&conn, &[]);

    let after = access_row(&conn, id);
    assert_eq!(before.0, 0);
    assert_eq!(after.0, 0, "empty id list must not bump any row");
    assert_eq!(
        before.1, after.1,
        "empty id list must not rewrite last_accessed"
    );
}

#[test]
fn insert_returns_pk_and_persists_columns() {
    let tmp = TempDir::new().expect("tempdir");
    let conn = connection::open(tmp.path().join("comemory.db")).expect("open db");

    let sid = code_row::insert(
        &conn,
        &CodeSymbolRow {
            repo: "myrepo",
            path: "src/lib.rs",
            blob_oid: "deadbeef",
            symbol: "do_work",
            kind: "function",
            lang: "rust",
            line_start: 10,
            line_end: 20,
            snippet: "fn do_work() {}",
            simhash: 42,
            parent_id: None,
        },
    )
    .expect("insert ok");

    let (repo, path, symbol, line_start, line_end, simhash, parent_id): (
        String,
        String,
        String,
        i64,
        i64,
        i64,
        Option<i64>,
    ) = conn
        .query_row(
            "SELECT repo, path, symbol, line_start, line_end, simhash, parent_id \
             FROM code_symbols WHERE id = ?1",
            rusqlite::params![sid],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .expect("row exists");

    assert_eq!(repo, "myrepo");
    assert_eq!(path, "src/lib.rs");
    assert_eq!(symbol, "do_work");
    assert_eq!(line_start, 10);
    assert_eq!(line_end, 20);
    assert_eq!(simhash, 42);
    assert_eq!(parent_id, None);
}

#[test]
fn insert_persists_parent_id_for_chunk_children() {
    let tmp = TempDir::new().expect("tempdir");
    let conn = connection::open(tmp.path().join("comemory.db")).expect("open db");

    let parent = code_row::insert(
        &conn,
        &CodeSymbolRow {
            repo: "myrepo",
            path: "src/lib.rs",
            blob_oid: "deadbeef",
            symbol: "big_fn",
            kind: "function",
            lang: "rust",
            line_start: 1,
            line_end: 90,
            snippet: "fn big_fn() {",
            simhash: 1,
            parent_id: None,
        },
    )
    .expect("insert parent");

    let child = code_row::insert(
        &conn,
        &CodeSymbolRow {
            repo: "myrepo",
            path: "src/lib.rs",
            blob_oid: "deadbeef",
            symbol: "big_fn#1",
            kind: "function",
            lang: "rust",
            line_start: 2,
            line_end: 60,
            snippet: "let x = 1;",
            simhash: 2,
            parent_id: Some(parent),
        },
    )
    .expect("insert child");

    let stored: Option<i64> = conn
        .query_row(
            "SELECT parent_id FROM code_symbols WHERE id = ?1",
            rusqlite::params![child],
            |r| r.get(0),
        )
        .expect("child row exists");
    assert_eq!(stored, Some(parent));
}
