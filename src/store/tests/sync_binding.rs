#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::store::sync_binding`].

use comemory::store::sync_binding;
use rusqlite::Connection;

fn open_with_binding_table() -> Connection {
    let conn = Connection::open_in_memory().expect("open");
    conn.execute_batch(
        "CREATE TABLE sync_binding (
            memory_id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL,
            secret_override_rule TEXT,
            secret_override_at TEXT
        );",
    )
    .expect("schema");
    conn
}

#[test]
fn bind_first_is_idempotent() {
    let conn = open_with_binding_table();
    sync_binding::bind_first(&conn, "abc12345", "ws-1").expect("bind");
    sync_binding::bind_first(&conn, "abc12345", "ws-2").expect("re-bind");
    let row = sync_binding::get(&conn, "abc12345")
        .expect("get")
        .expect("row");
    assert_eq!(row.workspace_id, "ws-1");
}

#[test]
fn allow_secret_records_override() {
    let conn = open_with_binding_table();
    sync_binding::allow_secret(
        &conn,
        "abc12345",
        "ws-1",
        "aws-access-key-id",
        "2026-01-01T00:00:00Z",
    )
    .expect("allow");
    assert!(sync_binding::has_secret_override(&conn, "abc12345").expect("check"));
}
