#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The acceptance decision itself — what is refused, and with which typed
//! disposition, before any state moves.

use crate::domains::code::replica_payload::{
    CODE_ENTITY_KIND, CODE_PAYLOAD_VERSION, CodeGenerationV1,
};
use crate::domains::sync::replica::contract::{CursorRef, Disposition, Operation};
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
    assert_eq!(
        crate::store::replica_redaction::redaction_of(&peer.conn, &digest).expect("redaction"),
        Some(replica_read::Redaction::Erased)
    );
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

// ---------------------------------------------------------------------------
// #251: an import cannot overwrite a local change this machine still owes.
// ---------------------------------------------------------------------------

#[test]
fn an_import_is_refused_while_this_machine_owes_a_change_to_the_same_memory() {
    // A real local save leaves a pending outbox row carrying its payload —
    // the only record of that edit until it is pushed.
    let mut home = Home::new();
    home.make_client();
    let id = home.save(BODY, &["sync"]);
    assert!(
        crate::store::replica_outbox::has_pending_for(&home.conn, "memory", &id)
            .expect("has pending"),
        "a local save owes an upload"
    );
    let pending_before = crate::store::replica_outbox::pending(&home.conn, 10).expect("pending");

    // A peer's version of the same memory arrives before the push happens.
    let mut peer = Home::new();
    let remote_id = peer.save(BODY, &["sync", "remote"]);
    assert_eq!(remote_id, id, "same body, same content-derived id");
    let remote = peer.payload(&remote_id);
    let operation = upsert("op-20260922-remote01", &remote);

    let mut ctx = home.ctx();
    let disposition = validate::decide(&mut ctx, &operation).expect("decide");

    assert_eq!(
        disposition,
        Disposition::RejectedStale,
        "the local edit is the only copy of itself; the peer waits for the push"
    );
    let pending_after = crate::store::replica_outbox::pending(&home.conn, 10).expect("pending");
    assert_eq!(
        pending_after, pending_before,
        "and the payload the outbox holds is untouched"
    );
}

#[test]
fn the_same_import_is_accepted_once_the_local_change_has_been_pushed() {
    let mut home = Home::new();
    let id = home.save(BODY, &["sync"]);
    let owed = crate::store::replica_outbox::pending(&home.conn, 10).expect("pending");
    assert_eq!(owed.len(), 1);

    // The push lands: the outbox row is answered, so nothing is owed for this
    // entity any more.
    crate::store::replica_outbox::record(
        &home.conn,
        &owed[0].operation_id,
        crate::store::replica_outbox::Outcome::Accepted {
            sequence: Some(7),
            disposition: "accepted",
            epoch: None,
        },
        "2026-09-22T10:00:00Z",
    )
    .expect("record the push");
    assert!(
        !crate::store::replica_outbox::has_pending_for(&home.conn, "memory", &id)
            .expect("has pending")
    );

    let mut peer = Home::new();
    let remote_id = peer.save(BODY, &["sync"]);
    let remote = peer.payload(&remote_id);
    let operation = upsert("op-20260922-remote02", &remote);
    let mut ctx = home.ctx();

    assert_ne!(
        validate::decide(&mut ctx, &operation).expect("decide"),
        Disposition::RejectedStale,
        "with nothing owed, ordinary revision ordering decides"
    );
}

#[test]
fn hub_accepts_an_import_for_an_entity_its_own_writes_left_pending() {
    // An engine nobody is a client of: its own save journalled (and queued) a
    // write, but it has no upstream to owe it to, so a client's edit of the
    // same memory is ordered, not refused.
    let mut home = Home::new();
    let id = home.save(BODY, &["sync"]);
    assert!(
        crate::store::replica_outbox::has_pending_for(&home.conn, "memory", &id)
            .expect("has pending")
    );
    let mut peer = Home::new();
    let remote_id = peer.save(BODY, &["sync", "remote"]);
    let operation = upsert(
        "op-20260924-hubaccepts000000000000000001",
        &peer.payload(&remote_id),
    );

    let mut ctx = home.ctx();
    assert_ne!(
        validate::decide(&mut ctx, &operation).expect("decide"),
        Disposition::RejectedStale,
        "the hub owes no upload, so nothing is refused on its account"
    );
}

#[test]
fn a_pending_change_to_one_memory_does_not_refuse_an_import_of_another() {
    let mut home = Home::new();
    home.save(BODY, &["sync"]);

    let mut peer = Home::new();
    let other_id = peer.save(
        "a different memory entirely: the writer reservation precedes the walk",
        &["sync"],
    );
    let other = peer.payload(&other_id);
    let operation = upsert("op-20260922-other001", &other);

    let mut ctx = home.ctx();
    assert_ne!(
        validate::decide(&mut ctx, &operation).expect("decide"),
        Disposition::RejectedStale,
        "the guard is per entity, not a global import freeze"
    );
}

// ---------------------------------------------------------------------------
// #252: the code_generation kind reaches the same ordering rules memories use.
// ---------------------------------------------------------------------------

/// An upsert carrying a real code generation for `repo`.
fn code_operation(operation_id: &str, repo: &str, payload: &CodeGenerationV1) -> Operation {
    let (bytes, digest) = payload.canonical().expect("canonical");
    Operation {
        operation_id: operation_id.to_string(),
        entity_kind: CODE_ENTITY_KIND.to_string(),
        entity_key: repo.to_string(),
        op: ReplicaOp::Upsert,
        schema_version: CODE_PAYLOAD_VERSION,
        payload_digest: Some(digest),
        payload: Some(serde_json::from_str(&bytes).expect("payload json")),
        observed_sequence: None,
        repository: Some(repo.to_string()),
        vector: None,
    }
}

/// A generation payload with one file, minted so it owns its id.
fn code_payload(head: &str) -> CodeGenerationV1 {
    let projection = crate::store::remote_code::Projection {
        files: vec![crate::store::remote_code::File {
            path: "src/lib.rs".to_string(),
            blob_oid: "aaaa1111".to_string(),
        }],
        symbols: Vec::new(),
        edges: Vec::new(),
    };
    let base = CodeGenerationV1::new("", None, head, None, &projection);
    let id = base.mint_id().expect("mint");
    base.with_id(&id)
}

#[test]
fn a_well_formed_code_generation_is_accepted() {
    let mut home = Home::new();
    let operation = code_operation(
        "op-20260922-code0001",
        "Falconiere/comemory",
        &code_payload("head-1"),
    );

    let mut ctx = home.ctx();
    let decided = validate::decide(&mut ctx, &operation).expect("decide");

    assert_eq!(
        decided,
        Disposition::Accepted,
        "the new kind reaches ordering instead of being refused as unsupported"
    );
}

#[test]
fn a_code_generation_whose_contents_were_edited_is_invalid() {
    let mut home = Home::new();
    let mut tampered = code_payload("head-1");
    // The id still claims the original contents.
    tampered.head = "a-different-head".to_string();
    let operation = code_operation("op-20260922-code0002", "Falconiere/comemory", &tampered);

    let mut ctx = home.ctx();
    let decided = validate::decide(&mut ctx, &operation).expect("decide");

    assert_eq!(
        decided,
        Disposition::RejectedInvalid,
        "a generation id is content-derived, so edited contents cannot keep it"
    );
}

#[test]
fn a_code_generation_on_an_unknown_schema_version_is_unsupported() {
    let mut home = Home::new();
    let mut operation = code_operation(
        "op-20260922-code0003",
        "Falconiere/comemory",
        &code_payload("head-1"),
    );
    operation.schema_version = CODE_PAYLOAD_VERSION + 1;

    let mut ctx = home.ctx();

    assert_eq!(
        validate::decide(&mut ctx, &operation).expect("decide"),
        Disposition::RejectedUnsupported,
        "an engine refuses a payload shape it cannot read rather than half-applying it"
    );
}

#[test]
fn a_pending_local_generation_refuses_an_incoming_one_for_the_same_repo() {
    let mut home = Home::new();
    home.make_client();
    let repo = "Falconiere/comemory";
    // The same guard memories get: an unpushed local change is the only copy
    // of itself, so a peer's version waits for the push.
    crate::store::replica_outbox::enqueue(
        &home.conn,
        &crate::store::replica_journal::NewOperation {
            operation_id: "op-20260922-localgen",
            entity_kind: CODE_ENTITY_KIND,
            entity_key: repo,
            op: ReplicaOp::Upsert,
            payload: None,
            schema_version: CODE_PAYLOAD_VERSION,
            repository: Some(repo),
            origin: crate::store::replica_journal::ReplicaOrigin::Local,
            at: "2026-09-22T10:00:00Z",
        },
        None,
    )
    .expect("enqueue");
    let operation = code_operation("op-20260922-code0004", repo, &code_payload("head-1"));

    let mut ctx = home.ctx();

    assert_eq!(
        validate::decide(&mut ctx, &operation).expect("decide"),
        Disposition::RejectedStale,
        "the guard is per entity and the repo IS the entity for this kind"
    );
}
