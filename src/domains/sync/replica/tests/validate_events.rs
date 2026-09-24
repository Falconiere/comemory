#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The event kinds' own acceptance rules, decided before any state moves.

use crate::domains::learning::feedback_tracking::record_with_provenance;
use crate::domains::learning::telemetry::StatsDb;
use crate::domains::sync::replica::contract::Disposition;
use crate::domains::sync::replica::validate;
use crate::domains::sync::replica::validate_events::is_event_kind;
use crate::store::replica_journal::ReplicaOp;
use crate::utilities::telemetry::PROV_MANUAL;

use crate::domains::sync::replica::test_support as support;

use support::{BODY, Home, approve, journalled_ops};

#[test]
fn only_the_two_event_kinds_take_these_rules() {
    assert!(is_event_kind("feedback_event") && is_event_kind("activity_event"));
    assert!(!is_event_kind("memory") && !is_event_kind("document_revision"));
}

#[test]
fn a_memory_target_must_name_a_real_memory_id() {
    let mut author = Home::new();
    approve(&author.conn, "Falconiere/comemory");
    let id = author.save(BODY, &["sync"]);
    let mut db = StatsDb::open(author.paths.stats_db()).expect("stats");
    record_with_provenance(&mut db, "q-20260924-1a2b3c4d", &[id], &[], PROV_MANUAL)
        .expect("verdict");
    let mut op = journalled_ops(&author.conn, "feedback_event").remove(0);
    let mut payload = op.payload.clone().expect("payload");
    payload["target"]["id"] = serde_json::json!("not-a-memory-id");
    let (_, digest) = crate::utilities::canonical_json::bytes_and_digest(&payload).expect("digest");
    op.payload = Some(payload);
    op.payload_digest = Some(digest);

    let mut peer = Home::new();
    let mut ctx = peer.ctx();
    assert_eq!(
        validate::decide(&mut ctx, &op).expect("decide"),
        Disposition::RejectedInvalid
    );

    op.op = ReplicaOp::Restore;
    let mut ctx = peer.ctx();
    assert_eq!(
        validate::decide(&mut ctx, &op).expect("decide"),
        Disposition::RejectedInvalid
    );
}
