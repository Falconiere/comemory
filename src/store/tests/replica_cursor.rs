#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::store::replica_cursor`] against a real migrated
//! database — one position per `(api_url, workspace)` key, carrying the epoch
//! it is valid under and the entry found at it.

use comemory::store::replica_cursor::{self, Anchor, Cursor};
use comemory::store::{connection, migrate};
use rusqlite::Connection;
use tempfile::TempDir;

const API: &str = "https://api.comemory.io";

fn migrated_db() -> (TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    migrate::run(&mut conn).expect("migrate");
    (dir, conn)
}

fn cursor(api: &str, workspace: &str, epoch: &str, sequence: i64) -> Cursor {
    Cursor {
        api_url: api.to_string(),
        workspace_id: workspace.to_string(),
        stream_epoch: epoch.to_string(),
        applied_sequence: sequence,
        anchor: None,
    }
}

#[test]
fn an_unknown_key_has_no_cursor() {
    let (_dir, conn) = migrated_db();
    assert!(
        replica_cursor::load(&conn, API, "ws_missing")
            .expect("load")
            .is_none()
    );
}

#[test]
fn saving_twice_advances_one_row_rather_than_accumulating() {
    let (_dir, conn) = migrated_db();
    let epoch = "0123456789abcdef0123456789abcdef";
    replica_cursor::save(
        &conn,
        &cursor(API, "ws_a", epoch, 10),
        "2026-09-21T10:00:00Z",
    )
    .expect("save");
    replica_cursor::save(
        &conn,
        &cursor(API, "ws_a", epoch, 42),
        "2026-09-21T10:05:00Z",
    )
    .expect("save");

    let loaded = replica_cursor::load(&conn, API, "ws_a")
        .expect("load")
        .expect("cursor");
    assert_eq!(loaded.applied_sequence, 42);
    assert_eq!(loaded.stream_epoch, epoch);
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM replica_cursor", [], |r| r.get(0))
        .expect("count");
    assert_eq!(
        rows, 1,
        "the second save advanced the row, it did not add one"
    );
}

#[test]
fn the_same_workspace_at_two_origins_keeps_two_positions() {
    let (_dir, conn) = migrated_db();
    let epoch = "0123456789abcdef0123456789abcdef";
    replica_cursor::save(
        &conn,
        &cursor(API, "ws_a", epoch, 10),
        "2026-09-21T10:00:00Z",
    )
    .expect("save a");
    replica_cursor::save(
        &conn,
        &cursor("http://127.0.0.1:9/api", "ws_a", epoch, 3),
        "2026-09-21T10:00:00Z",
    )
    .expect("save b");

    let first = replica_cursor::load(&conn, API, "ws_a")
        .expect("load a")
        .expect("cursor a");
    let second = replica_cursor::load(&conn, "http://127.0.0.1:9/api", "ws_a")
        .expect("load b")
        .expect("cursor b");
    assert_eq!(first.applied_sequence, 10);
    assert_eq!(
        second.applied_sequence, 3,
        "an origin never inherits another origin's progress for the same workspace"
    );
}

#[test]
fn the_anchor_round_trips_and_clears() {
    let (_dir, conn) = migrated_db();
    let epoch = "0123456789abcdef0123456789abcdef";
    let mut anchored = cursor(API, "ws_a", epoch, 17);
    anchored.anchor = Some(Anchor {
        sequence: 17,
        operation_id: "op-20260924-00112233445566778899aabbccddeeff".to_string(),
    });
    replica_cursor::save(&conn, &anchored, "2026-09-24T10:00:00Z").expect("save");
    let loaded = replica_cursor::load(&conn, API, "ws_a")
        .expect("load")
        .expect("cursor");
    assert_eq!(loaded.anchor, anchored.anchor);

    // A rewind to a position whose entry is unknown clears it rather than
    // leaving the old one to trip a false "stream rewritten".
    replica_cursor::save(
        &conn,
        &cursor(API, "ws_a", epoch, 5),
        "2026-09-24T10:01:00Z",
    )
    .expect("save");
    let rewound = replica_cursor::load(&conn, API, "ws_a")
        .expect("load")
        .expect("cursor");
    assert_eq!(rewound.anchor, None);
    assert_eq!(rewound.applied_sequence, 5);
}

#[test]
fn a_replaced_upstream_stores_its_new_epoch_with_the_reset_position() {
    let (_dir, conn) = migrated_db();
    replica_cursor::save(
        &conn,
        &cursor(API, "ws_a", "0123456789abcdef0123456789abcdef", 120),
        "2026-09-21T10:00:00Z",
    )
    .expect("save");

    // A restored upstream mints a new epoch; recovery rewrites both fields
    // together so a position can never be read under the wrong stream.
    replica_cursor::save(
        &conn,
        &cursor(API, "ws_a", "aaaabbbbccccddddeeeeffff00001111", 0),
        "2026-09-21T11:00:00Z",
    )
    .expect("save");

    let loaded = replica_cursor::load(&conn, API, "ws_a")
        .expect("load")
        .expect("cursor");
    assert_eq!(loaded.applied_sequence, 0);
    assert_eq!(loaded.stream_epoch, "aaaabbbbccccddddeeeeffff00001111");
}
