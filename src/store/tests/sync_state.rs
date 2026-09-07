#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::store::sync_state`].

use comemory::store::sync_state;
use rusqlite::Connection;

fn open_with_sync_state() -> Connection {
    let conn = Connection::open_in_memory().expect("open");
    conn.execute_batch(
        "CREATE TABLE sync_state (
            workspace_id TEXT PRIMARY KEY,
            api_url TEXT NOT NULL,
            pulled_seq INTEGER NOT NULL DEFAULT 0,
            pushed_seq INTEGER NOT NULL DEFAULT 0,
            last_sync_at TEXT
        );",
    )
    .expect("schema");
    conn
}

#[test]
fn ensure_and_get_roundtrip() {
    let conn = open_with_sync_state();
    sync_state::ensure(&conn, "ws-1", "https://api.example").expect("ensure");
    let row = sync_state::get(&conn, "ws-1").expect("get").expect("row");
    assert_eq!(row.api_url, "https://api.example");
}
