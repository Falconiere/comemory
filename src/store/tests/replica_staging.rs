#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::store::replica_staging`] against a real migrated
//! database — parts, re-sent parts, assembly order and the incomplete refusal.

use comemory::store::replica_staging;
use comemory::store::{connection, migrate};
use rusqlite::Connection;
use tempfile::TempDir;

fn migrated_db() -> (TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    migrate::run(&mut conn).expect("migrate");
    (dir, conn)
}

#[test]
fn an_incomplete_upload_assembles_to_nothing() {
    let (_dir, conn) = migrated_db();
    let state = replica_staging::put_part(
        &conn,
        "stage-1",
        0,
        2,
        r#"{"body":"fir"#,
        "2026-09-21T10:00:00Z",
    )
    .expect("part 0");

    assert_eq!(state.received, 1);
    assert_eq!(state.declared, 2);
    assert!(!state.complete());
    assert!(
        replica_staging::assemble(&conn, "stage-1")
            .expect("assemble")
            .is_none(),
        "a missing part must refuse assembly, not truncate the revision"
    );
}

#[test]
fn parts_assemble_in_index_order_regardless_of_arrival_order() {
    let (_dir, conn) = migrated_db();
    replica_staging::put_part(
        &conn,
        "stage-1",
        1,
        2,
        r#"st half"}"#,
        "2026-09-21T10:01:00Z",
    )
    .expect("part 1");
    let state = replica_staging::put_part(
        &conn,
        "stage-1",
        0,
        2,
        r#"{"body":"fir"#,
        "2026-09-21T10:00:00Z",
    )
    .expect("part 0");

    assert!(state.complete(), "both declared parts have arrived");
    assert_eq!(
        replica_staging::assemble(&conn, "stage-1").expect("assemble"),
        Some(r#"{"body":"first half"}"#.to_string())
    );
}

#[test]
fn a_resent_part_replaces_its_earlier_copy() {
    let (_dir, conn) = migrated_db();
    replica_staging::put_part(&conn, "stage-1", 0, 1, "stale", "2026-09-21T10:00:00Z")
        .expect("part");
    let state = replica_staging::put_part(&conn, "stage-1", 0, 1, "fresh", "2026-09-21T10:02:00Z")
        .expect("retry");

    assert_eq!(
        state.received, 1,
        "a retry is the same part, not a second one"
    );
    assert_eq!(
        replica_staging::assemble(&conn, "stage-1").expect("assemble"),
        Some("fresh".to_string())
    );
}

#[test]
fn discard_drops_one_upload_and_leaves_the_others() {
    let (_dir, conn) = migrated_db();
    replica_staging::put_part(&conn, "stage-1", 0, 1, "one", "2026-09-21T10:00:00Z").expect("a");
    replica_staging::put_part(&conn, "stage-2", 0, 1, "two", "2026-09-21T10:00:00Z").expect("b");

    assert_eq!(
        replica_staging::discard(&conn, "stage-1").expect("discard"),
        1
    );
    assert_eq!(
        replica_staging::assemble(&conn, "stage-1").expect("assemble"),
        None,
        "the discarded upload has nothing left to assemble"
    );
    assert_eq!(
        replica_staging::assemble(&conn, "stage-2").expect("assemble"),
        Some("two".to_string())
    );
}
