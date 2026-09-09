#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::store::sync_log`].

use comemory::store::sync_log::{self, SyncOp, SyncOrigin};
use rusqlite::Connection;

fn open_with_sync_log() -> Connection {
    let conn = Connection::open_in_memory().expect("open");
    conn.execute_batch(
        "CREATE TABLE sync_log (
            seq INTEGER PRIMARY KEY AUTOINCREMENT,
            op TEXT NOT NULL,
            memory_id TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            at TEXT NOT NULL,
            origin TEXT NOT NULL
        );",
    )
    .expect("schema");
    conn
}

#[test]
fn append_local_upsert_assigns_seq() {
    let conn = open_with_sync_log();
    let seq = sync_log::append(
        &conn,
        SyncOp::Upsert,
        "abc12345",
        "deadbeef",
        "2026-01-01T00:00:00Z",
        SyncOrigin::Local,
    )
    .expect("append");
    assert_eq!(seq, 1);
}

#[test]
fn empty_table_head_seq_is_zero() {
    let conn = open_with_sync_log();
    assert_eq!(sync_log::head_seq(&conn).expect("head"), 0);
}

#[test]
fn head_seq_tracks_max_after_appends() {
    let conn = open_with_sync_log();
    sync_log::append(
        &conn,
        SyncOp::Upsert,
        "aaaaaaaa",
        "11",
        "2026-01-01T00:00:00Z",
        SyncOrigin::Local,
    )
    .expect("1");
    sync_log::append(
        &conn,
        SyncOp::Tombstone,
        "aaaaaaaa",
        "11",
        "2026-01-02T00:00:00Z",
        SyncOrigin::Local,
    )
    .expect("2");
    assert_eq!(sync_log::head_seq(&conn).expect("head"), 2);
}

#[test]
fn latest_tombstone_seq_and_entries_since() {
    let conn = open_with_sync_log();
    sync_log::append(
        &conn,
        SyncOp::Upsert,
        "abcd1234",
        "aa",
        "2026-01-01T00:00:00Z",
        SyncOrigin::Local,
    )
    .expect("upsert");
    let tomb = sync_log::append(
        &conn,
        SyncOp::Tombstone,
        "abcd1234",
        "aa",
        "2026-01-02T00:00:00Z",
        SyncOrigin::Sync,
    )
    .expect("tomb");
    sync_log::append(
        &conn,
        SyncOp::Restore,
        "abcd1234",
        "bb",
        "2026-01-03T00:00:00Z",
        SyncOrigin::Local,
    )
    .expect("restore");

    assert_eq!(
        sync_log::latest_tombstone_seq(&conn, "abcd1234")
            .expect("tomb")
            .expect("some"),
        tomb
    );
    assert!(
        sync_log::latest_tombstone_seq(&conn, "missing")
            .expect("none")
            .is_none()
    );

    let since = sync_log::entries_since(&conn, 0, 10).expect("since");
    assert_eq!(since.len(), 3);
    assert_eq!(since[1].op, SyncOp::Tombstone);
    assert_eq!(since[1].origin, SyncOrigin::Sync);

    let local = sync_log::local_entries_since(&conn, 0, 10).expect("local");
    assert_eq!(local.len(), 2);
    assert!(local.iter().all(|r| r.origin == SyncOrigin::Local));
}

/// Pins the `seq > since` (strict) cursor boundary both `entries_since`
/// (pull) and `local_entries_since` (push) rely on: a row exactly AT the
/// stored cursor must not be re-selected, only the row strictly after it.
/// An off-by-one here would either re-push/re-pull an already-synced entry
/// (`>=`) or silently drop the very next one (`>` seeded one too high).
#[test]
fn entries_since_excludes_the_cursor_and_includes_the_next_seq() {
    let conn = open_with_sync_log();
    let at_cursor = sync_log::append(
        &conn,
        SyncOp::Upsert,
        "aaaaaaaa",
        "11",
        "2026-01-01T00:00:00Z",
        SyncOrigin::Local,
    )
    .expect("seed at cursor");
    let after_cursor = sync_log::append(
        &conn,
        SyncOp::Upsert,
        "bbbbbbbb",
        "22",
        "2026-01-02T00:00:00Z",
        SyncOrigin::Local,
    )
    .expect("seed after cursor");

    // From the origin both rows are visible, so the "1 row" below is a
    // filter result rather than an artifact of only one row existing.
    let all = sync_log::entries_since(&conn, 0, 10).expect("entries_since from origin");
    assert_eq!(all.len(), 2, "both seeded rows must be visible from seq 0");

    let rows = sync_log::entries_since(&conn, at_cursor, 10).expect("entries_since");
    assert_eq!(
        rows.len(),
        1,
        "a row exactly at the cursor must not be re-selected"
    );
    assert_eq!(rows[0].seq, after_cursor);

    let local = sync_log::local_entries_since(&conn, at_cursor, 10).expect("local_entries_since");
    assert_eq!(local.len(), 1);
    assert_eq!(local[0].seq, after_cursor);
}

#[test]
fn op_and_origin_parse_roundtrip() {
    assert_eq!(SyncOp::parse("upsert").unwrap(), SyncOp::Upsert);
    assert_eq!(SyncOp::parse("tombstone").unwrap(), SyncOp::Tombstone);
    assert_eq!(SyncOp::parse("restore").unwrap(), SyncOp::Restore);
    assert!(SyncOp::parse("nope").is_err());
    assert_eq!(SyncOp::Upsert.as_str(), "upsert");

    assert_eq!(SyncOrigin::parse("local").unwrap(), SyncOrigin::Local);
    assert_eq!(SyncOrigin::parse("sync").unwrap(), SyncOrigin::Sync);
    assert!(SyncOrigin::parse("nope").is_err());
}
