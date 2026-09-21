#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::store::replica_receipt`] against a real migrated
//! database — a decision is written once and read back unchanged.

use comemory::store::replica_receipt::{self, Receipt};
use comemory::store::{connection, migrate};
use rusqlite::Connection;
use tempfile::TempDir;

fn migrated_db() -> (TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    migrate::run(&mut conn).expect("migrate");
    (dir, conn)
}

fn accepted(operation_id: &str, sequence: i64) -> Receipt {
    Receipt {
        operation_id: operation_id.to_string(),
        epoch: "0123456789abcdef0123456789abcdef".to_string(),
        sequence: Some(sequence),
        disposition: "accepted".to_string(),
        payload_digest: Some("a".repeat(64)),
        reason: None,
    }
}

#[test]
fn an_unknown_operation_has_no_receipt() {
    let (_dir, conn) = migrated_db();
    assert!(
        replica_receipt::lookup(&conn, "op-20260921-deadbeef")
            .expect("lookup")
            .is_none()
    );
}

#[test]
fn a_recorded_decision_reads_back_with_its_original_position() {
    let (_dir, mut conn) = migrated_db();
    let tx = conn.transaction().expect("tx");
    replica_receipt::record(&tx, &accepted("op-1", 41), "2026-09-21T10:00:00Z").expect("record");
    tx.commit().expect("commit");

    let found = replica_receipt::lookup(&conn, "op-1")
        .expect("lookup")
        .expect("receipt");
    assert_eq!(found.sequence, Some(41));
    assert_eq!(found.disposition, "accepted");
    assert_eq!(
        found.payload_digest.as_deref(),
        Some("a".repeat(64).as_str())
    );
    assert_eq!(found.reason, None);
}

#[test]
fn a_refusal_keeps_its_reason_and_carries_no_position() {
    let (_dir, mut conn) = migrated_db();
    let tx = conn.transaction().expect("tx");
    replica_receipt::record(
        &tx,
        &Receipt {
            operation_id: "op-2".to_string(),
            epoch: "0123456789abcdef0123456789abcdef".to_string(),
            sequence: None,
            disposition: "rejected_stale".to_string(),
            payload_digest: Some("b".repeat(64)),
            reason: Some("tombstone at 7 is newer than observed 5".to_string()),
        },
        "2026-09-21T10:01:00Z",
    )
    .expect("record");
    tx.commit().expect("commit");

    let found = replica_receipt::lookup(&conn, "op-2")
        .expect("lookup")
        .expect("receipt");
    assert_eq!(found.sequence, None, "a refusal wrote no feed position");
    assert_eq!(found.disposition, "rejected_stale");
    assert_eq!(
        found.reason.as_deref(),
        Some("tombstone at 7 is newer than observed 5")
    );
}

#[test]
fn a_second_decision_for_one_operation_is_refused() {
    let (_dir, mut conn) = migrated_db();
    let tx = conn.transaction().expect("tx");
    replica_receipt::record(&tx, &accepted("op-3", 9), "2026-09-21T10:00:00Z").expect("first");
    tx.commit().expect("commit");

    let tx = conn.transaction().expect("tx");
    let second = replica_receipt::record(&tx, &accepted("op-3", 10), "2026-09-21T10:02:00Z");
    assert!(
        second.is_err(),
        "one operation has one decision; a replay reads it rather than replacing it"
    );
    drop(tx);

    let found = replica_receipt::lookup(&conn, "op-3")
        .expect("lookup")
        .expect("receipt");
    assert_eq!(found.sequence, Some(9), "the original position stands");
}
