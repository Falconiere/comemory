#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::store::replica_journal`] against a real migrated
//! database — the payload row, the feed append and the revision update one
//! accepted mutation owes.

use comemory::store::replica_journal::{
    self, NewOperation, PayloadRef, ReplicaOp, ReplicaOrigin, redact_payload, stream_epoch,
};
use comemory::store::replica_read;
use comemory::store::{connection, migrate};
use rusqlite::Connection;
use tempfile::TempDir;

/// A real database at the current schema, exactly as the runtime opens it.
fn migrated_db() -> (TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    migrate::run(&mut conn).expect("migrate");
    (dir, conn)
}

fn upsert<'a>(operation_id: &'a str, key: &'a str, payload: PayloadRef<'a>) -> NewOperation<'a> {
    NewOperation {
        operation_id,
        entity_kind: "memory",
        entity_key: key,
        op: ReplicaOp::Upsert,
        payload: Some(payload),
        schema_version: 1,
        repository: Some("Falconiere/comemory"),
        origin: ReplicaOrigin::Local,
        at: "2026-09-21T10:00:00Z",
    }
}

#[test]
fn append_stores_the_payload_assigns_a_sequence_and_points_the_revision_at_it() {
    let (_dir, mut conn) = migrated_db();
    let epoch = stream_epoch(&conn).expect("epoch");
    let digest_a = "a".repeat(64);
    let payload = PayloadRef {
        digest: &digest_a,
        bytes: r#"{"body":"server acceptance orders changes"}"#,
    };

    let tx = conn.transaction().expect("tx");
    let sequence =
        replica_journal::append(&tx, &epoch, &upsert("op-1", "a1b2c3d4", payload)).expect("append");
    tx.commit().expect("commit");

    assert_eq!(sequence, 1);
    let page = replica_read::page(&conn, 0, 10, None).expect("page");
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].sequence, 1);
    assert_eq!(page[0].payload.as_deref(), Some(payload.bytes));
    assert_eq!(page[0].entity_key, "a1b2c3d4");
    assert!(!page[0].payload_erased);

    let revision = replica_read::revision(&conn, "memory", "a1b2c3d4")
        .expect("revision")
        .expect("one revision");
    assert_eq!(revision.sequence, 1);
    assert_eq!(revision.payload_digest.as_deref(), Some(payload.digest));
    assert!(!revision.deleted);
    assert_eq!(revision.deleted_sequence, None);
}

#[test]
fn a_second_payload_with_the_same_digest_does_not_replace_the_stored_bytes() {
    let (_dir, mut conn) = migrated_db();
    let epoch = stream_epoch(&conn).expect("epoch");
    let digest_b = "b".repeat(64);
    let digest = digest_b.as_str();
    let first = PayloadRef {
        digest,
        bytes: r#"{"body":"first"}"#,
    };

    let tx = conn.transaction().expect("tx");
    replica_journal::append(&tx, &epoch, &upsert("op-1", "a1b2c3d4", first)).expect("first");
    tx.commit().expect("commit");

    // The same digest arriving again must not rewrite history: content
    // addressing is what makes a replay share the original payload row.
    let tx = conn.transaction().expect("tx");
    replica_journal::append(
        &tx,
        &epoch,
        &upsert(
            "op-2",
            "a1b2c3d4",
            PayloadRef {
                digest,
                bytes: r#"{"body":"second"}"#,
            },
        ),
    )
    .expect("second");
    tx.commit().expect("commit");

    let page = replica_read::page(&conn, 0, 10, None).expect("page");
    assert_eq!(page[0].payload.as_deref(), Some(r#"{"body":"first"}"#));
    assert!(!page[0].payload_erased);
    assert!(!replica_read::is_erased(&conn, digest).expect("erased"));
}

#[test]
fn a_tombstone_records_its_own_position_and_a_restore_clears_it() {
    let (_dir, mut conn) = migrated_db();
    let epoch = stream_epoch(&conn).expect("epoch");
    let digest_c = "c".repeat(64);
    let digest = digest_c.as_str();
    let payload = PayloadRef {
        digest,
        bytes: r#"{"body":"kept"}"#,
    };

    let tx = conn.transaction().expect("tx");
    replica_journal::append(&tx, &epoch, &upsert("op-1", "a1b2c3d4", payload)).expect("upsert");
    let tombstone_seq = replica_journal::append(
        &tx,
        &epoch,
        &NewOperation {
            operation_id: "op-2",
            op: ReplicaOp::Tombstone,
            payload: None,
            ..upsert("op-2", "a1b2c3d4", payload)
        },
    )
    .expect("tombstone");
    tx.commit().expect("commit");

    let deleted = replica_read::revision(&conn, "memory", "a1b2c3d4")
        .expect("revision")
        .expect("row");
    assert!(deleted.deleted);
    assert_eq!(deleted.deleted_sequence, Some(tombstone_seq));
    assert_eq!(deleted.payload_digest, None, "a tombstone holds no payload");

    let tx = conn.transaction().expect("tx");
    let restore_seq = replica_journal::append(
        &tx,
        &epoch,
        &NewOperation {
            operation_id: "op-3",
            op: ReplicaOp::Restore,
            ..upsert("op-3", "a1b2c3d4", payload)
        },
    )
    .expect("restore");
    tx.commit().expect("commit");

    let restored = replica_read::revision(&conn, "memory", "a1b2c3d4")
        .expect("revision")
        .expect("row");
    assert!(!restored.deleted);
    assert_eq!(restored.sequence, restore_seq);
    assert_eq!(restored.deleted_sequence, None);
    assert!(
        restore_seq > tombstone_seq,
        "a restore lands above its tombstone"
    );
}

#[test]
fn a_duplicate_operation_id_is_refused_rather_than_journalled_twice() {
    let (_dir, mut conn) = migrated_db();
    let epoch = stream_epoch(&conn).expect("epoch");
    let digest_d = "d".repeat(64);
    let payload = PayloadRef {
        digest: &digest_d,
        bytes: r#"{"body":"once"}"#,
    };

    let tx = conn.transaction().expect("tx");
    replica_journal::append(&tx, &epoch, &upsert("op-1", "a1b2c3d4", payload)).expect("first");
    tx.commit().expect("commit");

    let tx = conn.transaction().expect("tx");
    let again = replica_journal::append(&tx, &epoch, &upsert("op-1", "a1b2c3d4", payload));
    assert!(
        again.is_err(),
        "uq_replica_feed_operation must refuse a second position for one operation"
    );
    drop(tx);

    assert_eq!(replica_read::head(&conn).expect("head"), 1);
}

#[test]
fn redaction_blanks_the_bytes_and_keeps_the_barrier() {
    let (_dir, mut conn) = migrated_db();
    let epoch = stream_epoch(&conn).expect("epoch");
    let digest_e = "e".repeat(64);
    let digest = digest_e.as_str();
    let tx = conn.transaction().expect("tx");
    replica_journal::append(
        &tx,
        &epoch,
        &upsert(
            "op-1",
            "a1b2c3d4",
            PayloadRef {
                digest,
                bytes: r#"{"body":"erase me"}"#,
            },
        ),
    )
    .expect("append");
    tx.commit().expect("commit");

    let redacted = redact_payload(&conn, digest, "2026-09-21T11:00:00Z").expect("redact");
    assert_eq!(redacted, 1);

    assert!(
        replica_read::is_erased(&conn, digest).expect("erased"),
        "the row stays as the erasure barrier"
    );

    let page = replica_read::page(&conn, 0, 10, None).expect("page");
    assert_eq!(page[0].payload, None);
    assert!(page[0].payload_erased);
    assert_eq!(
        page[0].payload_digest.as_deref(),
        Some(digest),
        "the digest survives so an old upsert cannot resurrect the bytes"
    );

    // A second redaction is a no-op rather than a second erasure timestamp.
    assert_eq!(
        redact_payload(&conn, digest, "2026-09-21T12:00:00Z").expect("redact again"),
        0
    );
}

#[test]
fn stream_epoch_is_the_minted_value() {
    let (_dir, conn) = migrated_db();
    let epoch = stream_epoch(&conn).expect("epoch");
    assert_eq!(epoch.len(), 32);
    let stored: String = conn
        .query_row("SELECT epoch FROM replica_stream", [], |r| r.get(0))
        .expect("row");
    assert_eq!(epoch, stored);
}
