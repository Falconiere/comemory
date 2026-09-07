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
