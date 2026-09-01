#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! [`insert`] against a real migrated `comemory.db`.

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
