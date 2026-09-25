#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The upgrade step over a real database: the horizon is recorded, and a
//! document revision journalled before the outbox queued anything is queued
//! exactly once.

use crate::domains::sync::drain::upgrade;
use crate::domains::sync::replica::test_support::Home;
use crate::store::replica_journal::{self, NewOperation, PayloadRef, ReplicaOp, ReplicaOrigin};
use crate::store::replica_outbox::{self, Scope};
use crate::store::sync_exchange::{ExchangeKey, ExchangeRow};
use crate::store::sync_state;

#[test]
fn begin_records_the_horizon_and_queues_unqueued_revisions_once() {
    let home = Home::new();
    // A revision journalled the way #253 did it: a feed row, no outbox row.
    let epoch = home.epoch();
    let digest = "e".repeat(64);
    replica_journal::append(
        &home.conn,
        &epoch,
        &NewOperation {
            operation_id: "op-20260923-legacydocument00000000000000001",
            entity_kind: "document_revision",
            entity_key: &"0d".repeat(16),
            op: ReplicaOp::Upsert,
            payload: Some(PayloadRef {
                digest: &digest,
                bytes: "{}",
            }),
            schema_version: 1,
            repository: Some("falconiere/comemory"),
            origin: ReplicaOrigin::Local,
            at: "2026-09-23T10:00:00Z",
        },
    )
    .expect("journal");
    let mut row = ExchangeRow::fresh(&ExchangeKey::new("http://127.0.0.1:9/api", "ws"));
    // The old protocol delivered something for this workspace.
    sync_state::ensure(&home.conn, "ws", "http://127.0.0.1:9/api").expect("state");
    sync_state::set_pushed(&home.conn, "ws", 7, "t").expect("pushed");

    let adopted = upgrade::begin(&home.conn, &mut row, 412, "t").expect("begin");
    let again = upgrade::begin(&home.conn, &mut row, 412, "t").expect("again");

    assert_eq!(row.upgrade_through, Some(412));
    assert_eq!(adopted, 1);
    assert_eq!(again, 0, "a second upgrade queues nothing twice");
    let queued = replica_outbox::read(
        &home.conn,
        Scope::Operation("op-20260923-legacydocument00000000000000001"),
        1,
    )
    .expect("read");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].payload_digest.as_deref(), Some(digest.as_str()));
}

#[test]
fn a_workspace_the_old_protocol_never_delivered_to_has_no_horizon() {
    let home = Home::new();
    let mut row = ExchangeRow::fresh(&ExchangeKey::new("http://127.0.0.1:9/api", "ws"));
    // A legacy pull ran, but nothing was ever pushed.
    sync_state::ensure(&home.conn, "ws", "http://127.0.0.1:9/api").expect("state");

    upgrade::begin(&home.conn, &mut row, 412, "t").expect("begin");

    assert_eq!(
        row.upgrade_through, None,
        "nothing can have been delivered, so nothing waits for the horizon"
    );
}
