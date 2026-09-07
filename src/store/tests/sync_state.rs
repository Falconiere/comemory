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
    assert_eq!(row.pulled_seq, 0);
    assert_eq!(row.pushed_seq, 0);
}

#[test]
fn set_pulled_and_pushed_advance_cursors() {
    let conn = open_with_sync_state();
    sync_state::ensure(&conn, "ws-1", "https://api.example").expect("ensure");
    sync_state::set_pulled(&conn, "ws-1", 12, "2026-09-06T00:00:00Z").expect("pulled");
    sync_state::set_pushed(&conn, "ws-1", 9, "2026-09-06T01:00:00Z").expect("pushed");
    let row = sync_state::get(&conn, "ws-1").expect("get").expect("row");
    assert_eq!(row.pulled_seq, 12);
    assert_eq!(row.pushed_seq, 9);
    assert_eq!(row.last_sync_at.as_deref(), Some("2026-09-06T01:00:00Z"));
}

#[test]
fn list_orders_by_workspace_id() {
    let conn = open_with_sync_state();
    sync_state::ensure(&conn, "ws-b", "https://a").expect("b");
    sync_state::ensure(&conn, "ws-a", "https://a").expect("a");
    let rows = sync_state::list(&conn).expect("list");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].workspace_id, "ws-a");
    assert_eq!(rows[1].workspace_id, "ws-b");
}

#[test]
fn ensure_updates_api_url_on_conflict() {
    let conn = open_with_sync_state();
    sync_state::ensure(&conn, "ws-1", "https://old").expect("ensure");
    sync_state::set_pulled(&conn, "ws-1", 5, "2026-09-01T00:00:00Z").expect("stamp");
    sync_state::ensure(&conn, "ws-1", "https://new").expect("re-ensure");
    let row = sync_state::get(&conn, "ws-1").expect("get").expect("row");
    assert_eq!(row.api_url, "https://new");
    assert_eq!(row.pulled_seq, 5, "cursors must survive api_url rewrite");
}

#[test]
fn get_missing_returns_none() {
    let conn = open_with_sync_state();
    assert!(sync_state::get(&conn, "missing").expect("get").is_none());
}
