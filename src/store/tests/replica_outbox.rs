#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::store::replica_outbox`] against a real migrated
//! database — enqueue, drain order, settlement and retry accounting.

use comemory::store::replica_journal::{NewOperation, PayloadRef, ReplicaOp, ReplicaOrigin};
use comemory::store::replica_outbox::{self, Outcome};
use comemory::store::{connection, migrate};
use rusqlite::Connection;
use tempfile::TempDir;

fn migrated_db() -> (TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    migrate::run(&mut conn).expect("migrate");
    (dir, conn)
}

fn operation<'a>(id: &'a str, key: &'a str, digest: &'a str, at: &'a str) -> NewOperation<'a> {
    NewOperation {
        operation_id: id,
        entity_kind: "memory",
        entity_key: key,
        op: ReplicaOp::Upsert,
        payload: Some(PayloadRef {
            digest,
            bytes: r#"{"body":"payload"}"#,
        }),
        schema_version: 1,
        repository: Some("Falconiere/comemory"),
        origin: ReplicaOrigin::Local,
        at,
    }
}

#[test]
fn pending_drains_oldest_first_and_counts_what_is_owed() {
    let (_dir, mut conn) = migrated_db();
    let digest = "a".repeat(64);
    let tx = conn.transaction().expect("tx");
    replica_outbox::enqueue(
        &tx,
        &operation("op-2", "k2", &digest, "2026-09-21T10:05:00Z"),
        Some(4),
    )
    .expect("second");
    replica_outbox::enqueue(
        &tx,
        &operation("op-1", "k1", &digest, "2026-09-21T10:00:00Z"),
        Some(3),
    )
    .expect("first");
    tx.commit().expect("commit");

    let pending = replica_outbox::pending(&conn, 10).expect("pending");
    assert_eq!(
        pending
            .iter()
            .map(|p| p.operation_id.as_str())
            .collect::<Vec<_>>(),
        vec!["op-1", "op-2"],
        "the outbox drains in creation order, not insertion order"
    );
    assert_eq!(pending[0].observed_sequence, Some(3));
    assert_eq!(pending[0].op, ReplicaOp::Upsert);
    assert_eq!(pending[0].attempts, 0);
    assert_eq!(replica_outbox::pending_count(&conn).expect("count"), 2);

    let one = replica_outbox::pending(&conn, 1).expect("limited");
    assert_eq!(one.len(), 1, "the limit bounds one envelope");
}

#[test]
fn settling_an_operation_removes_it_from_what_is_owed_and_keeps_the_answer() {
    let (_dir, mut conn) = migrated_db();
    let digest = "b".repeat(64);
    let tx = conn.transaction().expect("tx");
    replica_outbox::enqueue(
        &tx,
        &operation("op-1", "k1", &digest, "2026-09-21T10:00:00Z"),
        None,
    )
    .expect("enqueue");
    tx.commit().expect("commit");

    let settled = replica_outbox::record(
        &conn,
        "op-1",
        Outcome::Accepted {
            sequence: Some(77),
            disposition: "accepted",
        },
        "2026-09-21T10:01:00Z",
    )
    .expect("record");
    assert_eq!(settled, 1);
    assert_eq!(replica_outbox::pending_count(&conn).expect("count"), 0);

    let (state, sequence, disposition): (String, Option<i64>, Option<String>) = conn
        .query_row(
            "SELECT state, upstream_sequence, disposition FROM replica_operation WHERE operation_id = 'op-1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("row");
    assert_eq!(state, "accepted");
    assert_eq!(sequence, Some(77));
    assert_eq!(disposition.as_deref(), Some("accepted"));
}

#[test]
fn a_rejection_keeps_the_row_for_diagnosis() {
    let (_dir, mut conn) = migrated_db();
    let digest = "c".repeat(64);
    let tx = conn.transaction().expect("tx");
    replica_outbox::enqueue(
        &tx,
        &operation("op-1", "k1", &digest, "2026-09-21T10:00:00Z"),
        None,
    )
    .expect("enqueue");
    tx.commit().expect("commit");

    replica_outbox::record(
        &conn,
        "op-1",
        Outcome::Rejected {
            disposition: "rejected_stale",
        },
        "2026-09-21T10:02:00Z",
    )
    .expect("record");

    assert_eq!(replica_outbox::pending_count(&conn).expect("count"), 0);
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM replica_operation", [], |r| r.get(0))
        .expect("count");
    assert_eq!(rows, 1, "a refusal is evidence, not something to delete");
}

#[test]
fn a_failed_attempt_is_counted_and_the_operation_stays_pending() {
    let (_dir, mut conn) = migrated_db();
    let digest = "d".repeat(64);
    let tx = conn.transaction().expect("tx");
    replica_outbox::enqueue(
        &tx,
        &operation("op-1", "k1", &digest, "2026-09-21T10:00:00Z"),
        None,
    )
    .expect("enqueue");
    tx.commit().expect("commit");

    for at in ["2026-09-21T10:03:00Z", "2026-09-21T10:04:00Z"] {
        replica_outbox::record(
            &conn,
            "op-1",
            Outcome::Failed {
                error: "connection reset",
            },
            at,
        )
        .expect("record");
    }

    let pending = replica_outbox::pending(&conn, 10).expect("pending");
    assert_eq!(
        pending.len(),
        1,
        "a transport failure does not drop the work"
    );
    assert_eq!(pending[0].attempts, 2);
    let error: Option<String> = conn
        .query_row(
            "SELECT last_error FROM replica_operation WHERE operation_id = 'op-1'",
            [],
            |r| r.get(0),
        )
        .expect("row");
    assert_eq!(error.as_deref(), Some("connection reset"));
}
