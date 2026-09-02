#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! [`insert`] against a real migrated `comemory.db`.

use comemory::store::gc_runs::GcRunRow;
use comemory::store::{connection, gc_runs};

#[test]
fn insert_writes_a_readable_row() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open + migrate");

    gc_runs::insert(
        &conn,
        "abcdef0123456789",
        "2026-08-31T10:05:00Z",
        3,
        40,
        12,
        4096,
    )
    .expect("insert gc_runs row");

    let (at, removed, log_rows, event_rows, bytes_freed): (String, i64, i64, i64, i64) = conn
        .query_row(
            "SELECT at, removed, log_rows, event_rows, bytes_freed FROM gc_runs WHERE id = ?1",
            ["abcdef0123456789"],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .expect("query row back");
    assert_eq!(at, "2026-08-31T10:05:00Z");
    assert_eq!(removed, 3);
    assert_eq!(log_rows, 40);
    assert_eq!(event_rows, 12);
    assert_eq!(bytes_freed, 4096);
}

#[test]
fn newest_on_an_empty_table_is_none() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open + migrate");

    assert_eq!(gc_runs::newest(&conn).expect("newest"), None);
}

#[test]
fn newest_returns_the_most_recent_row() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open + migrate");

    // Inserted oldest-last on purpose: `newest` must order by `at`, not by
    // insertion order or rowid.
    gc_runs::insert(
        &conn,
        "bbbbbbbbbbbbbbbb",
        "2026-08-30T12:00:00Z",
        7,
        70,
        17,
        2048,
    )
    .expect("insert newer");
    gc_runs::insert(
        &conn,
        "aaaaaaaaaaaaaaaa",
        "2026-08-01T00:00:00Z",
        1,
        2,
        3,
        4,
    )
    .expect("insert older");

    let row = gc_runs::newest(&conn).expect("newest").expect("a row");
    // The whole row, not a field at a time: a column added to `GcRunRow`
    // without a matching value here is then a compile error rather than a
    // silently unasserted field.
    assert_eq!(
        row,
        GcRunRow {
            id: "bbbbbbbbbbbbbbbb".to_string(),
            at: "2026-08-30T12:00:00Z".to_string(),
            removed: 7,
            log_rows: 70,
            event_rows: 17,
            bytes_freed: 2048,
        }
    );
}
