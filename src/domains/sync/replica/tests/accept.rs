#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Acceptance against a real database and a real markdown tree: what one
//! import does, what a replay does, and what the envelope itself may carry.

use crate::domains::memories::MemoryStore;
use crate::domains::sync::replica::accept;
use crate::domains::sync::replica::contract::{Disposition, MAX_OPERATIONS, PROTOCOL};
use crate::store::{replica_read, replica_receipt};

use crate::domains::sync::replica::test_support as support;

use support::{BODY, Home, envelope, envelope_at, tombstone, upsert};

/// A second home standing in for the peer that receives the operation.
fn peer_and_operation() -> (Home, crate::domains::sync::replica::contract::Operation) {
    let mut author = Home::new();
    let id = author.save(BODY, &["sync", "journal"]);
    let payload = author.payload(&id);
    (Home::new(), upsert("op-20260921-aaaaaaaa", &payload))
}

#[test]
fn an_accepted_operation_writes_markdown_state_a_position_and_a_receipt() {
    let (mut peer, operation) = peer_and_operation();
    let key = operation.entity_key.clone();
    let digest = operation.payload_digest.clone();

    let mut ctx = peer.ctx();
    let response = accept::run(&mut ctx, envelope(vec![operation])).expect("import");

    assert_eq!(response.protocol, PROTOCOL);
    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].disposition, Disposition::Accepted);
    assert_eq!(response.results[0].sequence, Some(1));
    assert_eq!(response.head_sequence, 1);

    let record = MemoryStore::new(peer.paths.clone())
        .load(&key)
        .expect("the memory exists on the peer");
    assert_eq!(record.body.trim_end(), BODY.trim_end());
    assert_eq!(
        record.frontmatter.author, "",
        "the accepting side stamps authorship; a peer cannot claim it"
    );

    let feed = replica_read::page(&peer.conn, 0, 10, None).expect("page");
    assert_eq!(feed.len(), 1);
    assert_eq!(feed[0].payload_digest, digest);
    let receipt = replica_receipt::lookup(&peer.conn, "op-20260921-aaaaaaaa")
        .expect("lookup")
        .expect("receipt");
    assert_eq!(receipt.sequence, Some(1));
    assert_eq!(receipt.disposition, "accepted");

    let mirrored: i64 = peer
        .conn
        .query_row("SELECT count(*) FROM memories WHERE id = ?1", [&key], |r| {
            r.get(0)
        })
        .expect("mirror row");
    assert_eq!(mirrored, 1);
}

#[test]
fn a_replay_returns_the_original_position_and_applies_nothing_new() {
    let (mut peer, operation) = peer_and_operation();
    let mut ctx = peer.ctx();
    let first = accept::run(&mut ctx, envelope(vec![operation.clone()])).expect("first");
    let mut ctx = peer.ctx();
    let second = accept::run(&mut ctx, envelope(vec![operation])).expect("replay");

    assert_eq!(first.results[0].disposition, Disposition::Accepted);
    assert_eq!(second.results[0].disposition, Disposition::Duplicate);
    assert_eq!(
        second.results[0].sequence, first.results[0].sequence,
        "a replay never receives a newer position"
    );
    assert_eq!(replica_read::head(&peer.conn).expect("head"), 1);
}

#[test]
fn reusing_an_operation_id_with_other_bytes_is_a_conflict_that_changes_nothing() {
    let (mut peer, operation) = peer_and_operation();
    let mut ctx = peer.ctx();
    accept::run(&mut ctx, envelope(vec![operation.clone()])).expect("first");

    let mut tampered = operation.clone();
    tampered.payload_digest = Some("f".repeat(64));

    let mut ctx = peer.ctx();
    let response = accept::run(&mut ctx, envelope(vec![tampered])).expect("conflicting");

    assert_eq!(
        response.results[0].disposition,
        Disposition::RejectedConflict
    );
    assert_eq!(response.results[0].sequence, None);
    assert_eq!(
        replica_read::head(&peer.conn).expect("head"),
        1,
        "the conflicting replay wrote no position"
    );
    let stored = MemoryStore::new(peer.paths.clone())
        .load(&operation.entity_key)
        .expect("load");
    assert_eq!(stored.body.trim_end(), BODY.trim_end());
}

#[test]
fn altered_bytes_under_the_original_digest_are_a_conflict_not_a_replay() {
    let (mut peer, operation) = peer_and_operation();
    let mut ctx = peer.ctx();
    accept::run(&mut ctx, envelope(vec![operation.clone()])).expect("first");

    // The digest is left untouched: a client resending altered bytes under the
    // original digest must not be answered "already applied".
    let mut lying = operation.clone();
    let mut payload = lying.payload.clone().expect("payload");
    payload["body"] = serde_json::json!("a different body entirely");
    lying.payload = Some(payload);

    let mut ctx = peer.ctx();
    let response = accept::run(&mut ctx, envelope(vec![lying])).expect("import");
    assert_eq!(
        response.results[0].disposition,
        Disposition::RejectedConflict
    );
    let stored = MemoryStore::new(peer.paths.clone())
        .load(&operation.entity_key)
        .expect("load");
    assert_eq!(stored.body.trim_end(), BODY.trim_end());
}

#[test]
fn a_tombstone_for_an_unknown_id_is_recorded_as_a_convergence_fact() {
    let mut peer = Home::new();
    let mut ctx = peer.ctx();
    let response =
        accept::run(&mut ctx, envelope(vec![tombstone("op-1", "a1b2c3d4")])).expect("import");

    assert_eq!(response.results[0].disposition, Disposition::Accepted);
    let revision = replica_read::revision(&peer.conn, "memory", "a1b2c3d4")
        .expect("revision")
        .expect("row");
    assert!(
        revision.deleted,
        "the deletion is remembered so a later pull cannot re-create it"
    );
}

#[test]
fn an_envelope_naming_a_workspace_is_refused_before_anything_applies() {
    let (mut peer, operation) = peer_and_operation();
    let mut request = envelope(vec![operation]);
    request.workspace_id = Some("ws_someone_elses".to_string());

    let mut ctx = peer.ctx();
    let refused = accept::run(&mut ctx, request);

    assert!(refused.is_err(), "workspace comes from the credential");
    assert_eq!(
        replica_read::head(&peer.conn).expect("head"),
        0,
        "nothing applied"
    );
}

#[test]
fn an_envelope_over_the_operation_cap_or_on_another_protocol_is_refused() {
    let (mut peer, operation) = peer_and_operation();

    let mut oversized = envelope(vec![operation.clone(); MAX_OPERATIONS + 1]);
    oversized
        .operations
        .iter_mut()
        .enumerate()
        .for_each(|(n, op)| {
            op.operation_id = format!("op-20260921-{n:08x}");
        });
    let mut ctx = peer.ctx();
    assert!(
        accept::run(&mut ctx, oversized).is_err(),
        "501 operations is over the cap"
    );

    let mut wrong_protocol = envelope(vec![operation]);
    wrong_protocol.protocol = "replica-v2".to_string();
    let mut ctx = peer.ctx();
    assert!(accept::run(&mut ctx, wrong_protocol).is_err());

    assert_eq!(replica_read::head(&peer.conn).expect("head"), 0);
}

#[test]
fn a_cursor_from_another_stream_fails_the_whole_envelope() {
    let (mut peer, operation) = peer_and_operation();
    let foreign = "0".repeat(32);
    let request = envelope_at(vec![operation], &foreign, 0);

    let mut ctx = peer.ctx();
    let refused = accept::run(&mut ctx, request);

    assert!(refused.is_err(), "a replaced stream must be visible");
    assert_eq!(replica_read::head(&peer.conn).expect("head"), 0);
}

#[test]
fn a_cursor_from_this_stream_is_accepted() {
    let (mut peer, operation) = peer_and_operation();
    let epoch = peer.epoch();
    let request = envelope_at(vec![operation], &epoch, 0);

    let mut ctx = peer.ctx();
    let response = accept::run(&mut ctx, request).expect("import");
    assert_eq!(response.results[0].disposition, Disposition::Accepted);
    assert_eq!(response.stream_epoch, epoch);
}
