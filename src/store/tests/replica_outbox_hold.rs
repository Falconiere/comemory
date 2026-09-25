#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::store::replica_outbox_hold`] and the eligible read of
//! [`comemory::store::replica_outbox`], against a real migrated database: a
//! held row stays pending but leaves the eligible queue, stamps and wire
//! fields persist, and the counts separate every state.

use comemory::store::connection;
use comemory::store::replica_journal::{NewOperation, ReplicaOp, ReplicaOrigin};
use comemory::store::replica_outbox::{self, Outcome, Scope};
use comemory::store::replica_outbox_hold::{self, Change, Hold, Target};
use comemory::store::sync_exchange::ExchangeKey;
use rusqlite::Connection;

fn db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

/// Enqueue tombstones (no payload needed) for `keys`, all at the SAME instant,
/// so only the insertion order can order them.
fn enqueue(conn: &mut Connection, keys: &[&str]) {
    let tx = conn.transaction().expect("tx");
    for key in keys {
        replica_outbox::enqueue(
            &tx,
            &NewOperation {
                operation_id: &format!("op-{key}"),
                entity_kind: "memory",
                entity_key: key,
                op: ReplicaOp::Tombstone,
                payload: None,
                schema_version: 1,
                repository: Some("comemory"),
                origin: ReplicaOrigin::Local,
                at: "2026-09-24T10:00:00Z",
            },
            None,
        )
        .expect("enqueue");
    }
    tx.commit().expect("commit");
}

#[test]
fn eligible_skips_held_rows_and_keeps_insertion_order_on_a_tie() {
    let (_dir, mut conn) = db();
    enqueue(&mut conn, &["zz", "aa", "mm"]);
    replica_outbox_hold::update(
        &conn,
        Target::Operation("op-aa"),
        Change::Hold(Some((Hold::Policy, "acme/private"))),
        "t",
    )
    .expect("hold");

    let eligible: Vec<String> = replica_outbox::read(&conn, Scope::Eligible, 10)
        .expect("eligible")
        .into_iter()
        .map(|r| r.operation_id)
        .collect();
    assert_eq!(
        eligible,
        vec!["op-zz", "op-mm"],
        "the order made, held row skipped"
    );
    let all = replica_outbox::pending(&conn, 10).expect("pending");
    assert_eq!(all.len(), 3, "a held row is still pending");
    assert_eq!(all[1].hold_reason.as_deref(), Some("policy"));
    assert_eq!(all[1].hold_detail.as_deref(), Some("acme/private"));

    replica_outbox_hold::update(&conn, Target::Operation("op-aa"), Change::Hold(None), "t")
        .expect("clear");
    assert_eq!(
        replica_outbox::read(&conn, Scope::Eligible, 10)
            .expect("eligible")
            .len(),
        3
    );
}

#[test]
fn stamps_and_wire_fields_persist_and_unstamped_rows_take_the_outgoing_key() {
    let (_dir, mut conn) = db();
    enqueue(&mut conn, &["a1", "b2"]);
    let current = ExchangeKey::new("http://127.0.0.1:9/api", "ws_a");
    let leaving = ExchangeKey::new("http://127.0.0.1:9/api", "ws_old");
    replica_outbox_hold::update(
        &conn,
        Target::Operation("op-a1"),
        Change::Stamp(&current),
        "t",
    )
    .expect("stamp");
    replica_outbox_hold::update(
        &conn,
        Target::Operation("op-a1"),
        Change::Wire {
            repository: Some("falconiere/comemory"),
            observed_sequence: Some(41),
        },
        "t",
    )
    .expect("wire");

    let stamped = replica_outbox_hold::update(
        &conn,
        Target::Unstamped(Some("2026-09-24T10:00:00Z")),
        Change::Stamp(&leaving),
        "t",
    )
    .expect("stamp unstamped");
    assert_eq!(stamped, 1, "only the row no send had stamped yet");

    let rows = replica_outbox::pending(&conn, 10).expect("pending");
    let a1 = rows.iter().find(|r| r.operation_id == "op-a1").expect("a1");
    let b2 = rows.iter().find(|r| r.operation_id == "op-b2").expect("b2");
    assert_eq!(a1.workspace_id.as_deref(), Some("ws_a"));
    assert_eq!(a1.wire_repository.as_deref(), Some("falconiere/comemory"));
    assert_eq!(a1.observed_sequence, Some(41));
    assert_eq!(b2.workspace_id.as_deref(), Some("ws_old"));

    let none_after = replica_outbox_hold::update(
        &conn,
        Target::Unstamped(Some("2026-09-24T09:00:00Z")),
        Change::Stamp(&current),
        "t",
    )
    .expect("nothing that old");
    assert_eq!(none_after, 0);
}

#[test]
fn counts_separate_pending_retryable_held_and_rejected() {
    let (_dir, mut conn) = db();
    enqueue(&mut conn, &["p1", "p2", "h1", "h2", "r1", "a1"]);
    replica_outbox::record(&conn, "op-p2", Outcome::Failed { error: "HTTP 502" }, "t")
        .expect("fail");
    replica_outbox_hold::update(
        &conn,
        Target::Operation("op-h1"),
        Change::Hold(Some((Hold::Secret, "aws-key"))),
        "t",
    )
    .expect("h1");
    replica_outbox_hold::update(
        &conn,
        Target::Operation("op-h2"),
        Change::Hold(Some((Hold::Incompatible, "x@2"))),
        "t",
    )
    .expect("h2");
    replica_outbox::record(
        &conn,
        "op-r1",
        Outcome::Rejected {
            disposition: "rejected_stale",
        },
        "t",
    )
    .expect("reject");
    replica_outbox::record(
        &conn,
        "op-a1",
        Outcome::Accepted {
            sequence: Some(9),
            disposition: "accepted",
            epoch: Some("0123456789abcdef0123456789abcdef"),
        },
        "t",
    )
    .expect("accept");

    let counts = replica_outbox_hold::counts(&conn).expect("counts");
    assert_eq!(counts.pending, 1, "p1: never attempted");
    assert_eq!(
        counts.retryable, 1,
        "p2: attempted, no answer — counted once, not also pending"
    );
    assert_eq!(counts.rejected, 1);
    let held: Vec<(Hold, i64)> = counts.held.into_iter().filter(|(_, n)| *n > 0).collect();
    assert_eq!(held, vec![(Hold::Secret, 1), (Hold::Incompatible, 1)]);
    assert_eq!(
        replica_outbox::read(&conn, Scope::Operation("op-a1"), 1)
            .expect("lookup")
            .pop()
            .expect("ours")
            .state,
        "accepted"
    );
    assert!(
        replica_outbox::read(&conn, Scope::Operation("op-someone-else"), 1)
            .expect("lookup")
            .is_empty()
    );
}

#[test]
fn an_unknown_hold_literal_is_an_error() {
    assert!(Hold::parse("bored").is_err());
    for hold in Hold::ALL {
        assert_eq!(Hold::parse(hold.as_str()).expect("round trip"), hold);
    }
}
