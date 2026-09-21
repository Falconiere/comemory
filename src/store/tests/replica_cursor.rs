#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::store::replica_cursor`] against a real migrated
//! database — one position per workspace, carrying the epoch it is valid
//! under.

use comemory::store::replica_cursor::{self, Cursor};
use comemory::store::{connection, migrate};
use rusqlite::Connection;
use tempfile::TempDir;

fn migrated_db() -> (TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    migrate::run(&mut conn).expect("migrate");
    (dir, conn)
}

fn cursor(workspace: &str, epoch: &str, sequence: i64) -> Cursor {
    Cursor {
        workspace_id: workspace.to_string(),
        api_url: "https://api.comemory.io".to_string(),
        stream_epoch: epoch.to_string(),
        applied_sequence: sequence,
    }
}

#[test]
fn an_unknown_workspace_has_no_cursor() {
    let (_dir, conn) = migrated_db();
    assert!(
        replica_cursor::load(&conn, "ws_missing")
            .expect("load")
            .is_none()
    );
    assert!(replica_cursor::all(&conn).expect("all").is_empty());
}

#[test]
fn saving_twice_advances_one_row_rather_than_accumulating() {
    let (_dir, conn) = migrated_db();
    let epoch = "0123456789abcdef0123456789abcdef";
    replica_cursor::save(&conn, &cursor("ws_a", epoch, 10), "2026-09-21T10:00:00Z").expect("save");
    replica_cursor::save(&conn, &cursor("ws_a", epoch, 42), "2026-09-21T10:05:00Z").expect("save");

    let loaded = replica_cursor::load(&conn, "ws_a")
        .expect("load")
        .expect("cursor");
    assert_eq!(loaded.applied_sequence, 42);
    assert_eq!(loaded.stream_epoch, epoch);
    assert_eq!(replica_cursor::all(&conn).expect("all").len(), 1);
}

#[test]
fn two_workspaces_keep_independent_positions_and_epochs() {
    let (_dir, conn) = migrated_db();
    replica_cursor::save(
        &conn,
        &cursor("ws_a", "0123456789abcdef0123456789abcdef", 10),
        "2026-09-21T10:00:00Z",
    )
    .expect("save a");
    replica_cursor::save(
        &conn,
        &cursor("ws_b", "fedcba9876543210fedcba9876543210", 3),
        "2026-09-21T10:00:00Z",
    )
    .expect("save b");

    let all = replica_cursor::all(&conn).expect("all");
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].workspace_id, "ws_a");
    assert_eq!(all[0].applied_sequence, 10);
    assert_eq!(all[1].workspace_id, "ws_b");
    assert_eq!(all[1].applied_sequence, 3);
    assert_ne!(all[0].stream_epoch, all[1].stream_epoch);
}

#[test]
fn a_replaced_upstream_stores_its_new_epoch_with_the_reset_position() {
    let (_dir, conn) = migrated_db();
    replica_cursor::save(
        &conn,
        &cursor("ws_a", "0123456789abcdef0123456789abcdef", 120),
        "2026-09-21T10:00:00Z",
    )
    .expect("save");

    // A restored upstream mints a new epoch; recovery rewrites both fields
    // together so a position can never be read under the wrong stream.
    replica_cursor::save(
        &conn,
        &cursor("ws_a", "aaaabbbbccccddddeeeeffff00001111", 0),
        "2026-09-21T11:00:00Z",
    )
    .expect("save");

    let loaded = replica_cursor::load(&conn, "ws_a")
        .expect("load")
        .expect("cursor");
    assert_eq!(loaded.applied_sequence, 0);
    assert_eq!(loaded.stream_epoch, "aaaabbbbccccddddeeeeffff00001111");
}
