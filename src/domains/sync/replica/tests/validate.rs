#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The acceptance decision itself — what is refused, and with which typed
//! disposition, before any state moves.

use crate::domains::sync::replica::contract::{CursorRef, Disposition};
use crate::domains::sync::replica::{accept, validate};
use crate::store::replica_journal::ReplicaOp;
use crate::store::{replica_journal, replica_read};

use crate::domains::sync::replica::test_support as support;

use support::{BODY, Home, envelope, tombstone, upsert};

/// A peer home plus one upsert operation built from a real saved memory.
fn peer_and_operation() -> (Home, crate::domains::sync::replica::contract::Operation) {
    let mut author = Home::new();
    let id = author.save(BODY, &["sync"]);
    let payload = author.payload(&id);
    (Home::new(), upsert("op-20260921-aaaaaaaa", &payload))
}

#[test]
fn an_unknown_entity_kind_or_schema_version_is_unsupported() {
    let (mut peer, operation) = peer_and_operation();

    let mut future_kind = operation.clone();
    future_kind.entity_kind = "document".to_string();
    let mut ctx = peer.ctx();
    assert_eq!(
        validate::decide(&mut ctx, &future_kind).expect("decide"),
        Disposition::RejectedUnsupported
    );

    let mut future_schema = operation;
    future_schema.schema_version = 99;
    let mut ctx = peer.ctx();
    assert_eq!(
        validate::decide(&mut ctx, &future_schema).expect("decide"),
        Disposition::RejectedUnsupported
    );
}

#[test]
fn a_digest_that_does_not_cover_the_payload_is_invalid() {
    let (mut peer, mut operation) = peer_and_operation();
    operation.payload_digest = Some("a".repeat(64));

    let mut ctx = peer.ctx();
    assert_eq!(
        validate::decide(&mut ctx, &operation).expect("decide"),
        Disposition::RejectedInvalid
    );
}

#[test]
fn a_payload_whose_body_no_longer_hashes_to_its_id_is_invalid() {
    let (mut peer, operation) = peer_and_operation();
    let mut payload: serde_json::Value = operation.payload.clone().expect("payload");
    payload["body"] = serde_json::Value::String("a different body entirely".to_string());
    let (bytes, digest) =
        crate::utilities::canonical_json::bytes_and_digest(&payload).expect("canonical");
    let mut tampered = operation;
    tampered.payload = Some(serde_json::from_slice(&bytes).expect("json"));
    tampered.payload_digest = Some(digest);

    let mut ctx = peer.ctx();
    assert_eq!(
        validate::decide(&mut ctx, &tampered).expect("decide"),
        Disposition::RejectedInvalid,
        "content-derived identity holds on the replica path too"
    );
}

#[test]
fn an_upsert_that_never_saw_the_deletion_cannot_revive_it() {
    let (mut peer, operation) = peer_and_operation();
    let key = operation.entity_key.clone();
    let mut ctx = peer.ctx();
    accept::run(&mut ctx, envelope(vec![operation.clone()])).expect("upsert");
    let mut ctx = peer.ctx();
    accept::run(&mut ctx, envelope(vec![tombstone("op-2", &key)])).expect("tombstone");

    let mut stale = operation;
    stale.operation_id = "op-3".to_string();
    let mut ctx = peer.ctx();
    assert_eq!(
        validate::decide(&mut ctx, &stale).expect("decide"),
        Disposition::RejectedStale
    );
}

#[test]
fn a_restore_must_name_the_deletion_it_observed() {
    let (mut peer, operation) = peer_and_operation();
    let key = operation.entity_key.clone();
    let mut ctx = peer.ctx();
    accept::run(&mut ctx, envelope(vec![operation.clone()])).expect("upsert");
    let mut ctx = peer.ctx();
    let deleted =
        accept::run(&mut ctx, envelope(vec![tombstone("op-2", &key)])).expect("tombstone");
    let deletion_sequence = deleted.results[0].sequence.expect("sequence");

    let mut blind_restore = operation.clone();
    blind_restore.operation_id = "op-3".to_string();
    blind_restore.op = ReplicaOp::Restore;
    let mut ctx = peer.ctx();
    assert_eq!(
        validate::decide(&mut ctx, &blind_restore).expect("decide"),
        Disposition::RejectedStale,
        "a restore that names no deletion is a blind overwrite"
    );

    let mut informed_restore = blind_restore;
    informed_restore.operation_id = "op-4".to_string();
    informed_restore.observed_sequence = Some(deletion_sequence);
    let mut ctx = peer.ctx();
    assert_eq!(
        validate::decide(&mut ctx, &informed_restore).expect("decide"),
        Disposition::Accepted
    );
}

#[test]
fn an_erased_payload_is_refused_with_its_own_disposition() {
    let (mut peer, operation) = peer_and_operation();
    let mut ctx = peer.ctx();
    accept::run(&mut ctx, envelope(vec![operation.clone()])).expect("upsert");

    let digest = operation.payload_digest.clone().expect("digest");
    replica_journal::redact_payload(&peer.conn, &digest, "2026-09-21T12:00:00Z").expect("redact");

    let mut resend = operation;
    resend.operation_id = "op-later".to_string();
    let mut ctx = peer.ctx();
    assert_eq!(
        validate::decide(&mut ctx, &resend).expect("decide"),
        Disposition::PayloadErased,
        "the barrier stands even though the bytes are gone"
    );
    assert!(replica_read::is_erased(&peer.conn, &digest).expect("erased"));
}

#[test]
fn a_cursor_is_only_valid_under_the_stream_it_was_taken_from() {
    let epoch = "0123456789abcdef0123456789abcdef";
    assert!(validate::check_cursor(None, epoch).is_ok());
    assert!(
        validate::check_cursor(
            Some(&CursorRef {
                stream_epoch: epoch.to_string(),
                sequence: 12,
            }),
            epoch
        )
        .is_ok()
    );
    let foreign = validate::check_cursor(
        Some(&CursorRef {
            stream_epoch: "ffffffffffffffffffffffffffffffff".to_string(),
            sequence: 12,
        }),
        epoch,
    );
    assert!(foreign.is_err(), "a replaced stream is not agreement");
}

#[test]
fn a_stored_disposition_round_trips_and_an_unknown_one_reads_as_invalid() {
    for disposition in [
        Disposition::Accepted,
        Disposition::Duplicate,
        Disposition::RejectedStale,
        Disposition::RejectedConflict,
        Disposition::RejectedUnsupported,
        Disposition::RejectedNotAllowed,
        Disposition::PayloadExpired,
        Disposition::PayloadErased,
    ] {
        assert_eq!(
            validate::parse_disposition(disposition.as_str()),
            disposition
        );
    }
    assert_eq!(
        validate::parse_disposition("something_a_newer_engine_wrote"),
        Disposition::RejectedInvalid
    );
}
