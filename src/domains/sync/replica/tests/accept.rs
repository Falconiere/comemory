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
fn a_replay_whose_declared_digest_is_wrong_still_replays_the_original_answer() {
    let (mut peer, operation) = peer_and_operation();
    let mut ctx = peer.ctx();
    let first = accept::run(&mut ctx, envelope(vec![operation.clone()])).expect("first");

    // Same operation id, same BYTES, a wrong declared digest. The bytes are
    // the identity, so this is a retry with a bad claim — not new content —
    // and it must not produce a second effect.
    let mut misdeclared = operation.clone();
    misdeclared.payload_digest = Some("f".repeat(64));

    let mut ctx = peer.ctx();
    let response = accept::run(&mut ctx, envelope(vec![misdeclared])).expect("replay");

    assert_eq!(response.results[0].disposition, Disposition::Duplicate);
    assert_eq!(
        response.results[0].sequence, first.results[0].sequence,
        "the original position stands"
    );
    assert_eq!(
        replica_read::head(&peer.conn).expect("head"),
        1,
        "the replay wrote no second position"
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

#[test]
fn a_refused_operation_replays_as_the_same_refusal() {
    let (mut peer, operation) = peer_and_operation();

    // A payload whose declared digest does not cover its bytes: refused as
    // invalid, and the refusal is remembered.
    let mut lying = operation;
    lying.payload_digest = Some("a".repeat(64));

    let mut ctx = peer.ctx();
    let first = accept::run(&mut ctx, envelope(vec![lying.clone()])).expect("first");
    assert_eq!(
        first.results[0].disposition,
        Disposition::RejectedInvalid,
        "the claim disagrees with the bytes"
    );

    let mut ctx = peer.ctx();
    let replay = accept::run(&mut ctx, envelope(vec![lying])).expect("replay");
    assert_eq!(
        replay.results[0].disposition,
        Disposition::RejectedInvalid,
        "the retry reads back the original answer, not a fresh conflict"
    );
    assert_eq!(replica_read::head(&peer.conn).expect("head"), 0);
}

// ---------------------------------------------------------------------------
// #251: the peer's embedding rides alongside the payload, never inside it.
// ---------------------------------------------------------------------------

#[test]
fn the_same_operation_hashes_identically_with_and_without_its_vector() {
    let mut author = Home::new();
    let id = author.save(BODY, &["sync"]);
    let payload = author.payload(&id);

    let plain = upsert("op-20260922-aaaaaaaa", &payload);
    let vectored = support::upsert_with_vector(
        "op-20260922-aaaaaaaa",
        &payload,
        support::wire_vector("bge-small-en-v1.5", 1024),
    );

    assert_eq!(
        plain.payload, vectored.payload,
        "the embedding is not part of the payload"
    );
    assert_eq!(
        plain.payload_digest, vectored.payload_digest,
        "so re-embedding a memory cannot mint a new revision"
    );
}

#[test]
fn an_accepted_operation_carrying_a_usable_vector_stores_it_once() {
    let mut author = Home::new();
    let id = author.save(BODY, &["sync"]);
    let payload = author.payload(&id);
    let mut peer = Home::new();
    let model = crate::store::schema_meta::memory_vector_model(&peer.conn).expect("model");
    let operation = support::upsert_with_vector(
        "op-20260922-vector01",
        &payload,
        support::wire_vector(&model, 1024),
    );

    let mut ctx = peer.ctx();
    let first = accept::run(&mut ctx, envelope(vec![operation.clone()])).expect("accept");
    assert_eq!(first.results[0].disposition, Disposition::Accepted);
    let mut ctx = peer.ctx();
    let replay = accept::run(&mut ctx, envelope(vec![operation])).expect("replay");
    assert_eq!(
        replay.results[0].disposition,
        Disposition::Duplicate,
        "the second delivery reads back its receipt rather than re-applying"
    );

    assert!(
        crate::store::vector::memory_embedding_blob(&peer.conn, &id)
            .expect("blob")
            .is_some(),
        "the peer's embedding landed"
    );
    let rows: i64 = peer
        .conn
        .query_row("SELECT count(*) FROM memory_vec", [], |r| r.get(0))
        .expect("count");
    assert_eq!(rows, 1, "a replay stores no second row");
    assert_eq!(
        crate::store::needs_embedding::pending(&peer.conn)
            .expect("pending")
            .len(),
        0
    );
    assert_eq!(
        replica_read::head(&peer.conn).expect("head"),
        1,
        "and one acceptance, not two"
    );
}

#[test]
fn a_foreign_vector_still_stores_the_memory_and_records_the_backlog() {
    let mut author = Home::new();
    let id = author.save(BODY, &["sync"]);
    let payload = author.payload(&id);
    let mut peer = Home::new();
    let operation = support::upsert_with_vector(
        "op-20260922-foreign1",
        &payload,
        support::wire_vector("text-embedding-3-small", 1024),
    );

    let mut ctx = peer.ctx();
    let response = accept::run(&mut ctx, envelope(vec![operation])).expect("accept");

    assert_eq!(
        response.results[0].disposition,
        Disposition::Accepted,
        "an unusable vector must never refuse the memory"
    );
    assert!(
        MemoryStore::new(peer.paths.clone()).load(&id).is_ok(),
        "the text is on disk"
    );
    let pending = crate::store::needs_embedding::pending(&peer.conn).expect("pending");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].memory_id, id);
    assert_eq!(
        pending[0].reason,
        crate::store::needs_embedding::Reason::Model
    );
    assert_eq!(pending[0].model.as_deref(), Some("text-embedding-3-small"));
}

#[test]
fn an_operation_with_no_vector_records_the_memory_as_needing_one() {
    let (mut peer, operation) = peer_and_operation();
    let entity_key = operation.entity_key.clone();

    let mut ctx = peer.ctx();
    accept::run(&mut ctx, envelope(vec![operation])).expect("accept");

    let pending = crate::store::needs_embedding::pending(&peer.conn).expect("pending");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].memory_id, entity_key);
    assert_eq!(
        pending[0].reason,
        crate::store::needs_embedding::Reason::Absent,
        "the peer sent no vector, which is a backlog entry rather than a failure"
    );
}

#[test]
fn the_digest_an_acceptance_answers_is_the_digest_it_stores() {
    // Both engines wrote the same body independently, so the receiving one
    // already holds the memory under its own creation time. Copying every
    // replicated field EXCEPT `created` left it acknowledging one digest and
    // storing another, and two such engines could never agree on a manifest
    // digest however often they imported from one another.
    let mut author = Home::new();
    let id = author.save(BODY, &["sync", "drifted"]);
    let payload = author.payload(&id);
    let mut peer = Home::new();
    let peer_id = peer.save(BODY, &["sync"]);
    assert_eq!(peer_id, id, "same body, same content-derived id");
    support::mark_pushed(&peer.conn, &id);

    let operation = upsert("op-20260922-identity", &payload);
    let answered = {
        let mut ctx = peer.ctx();
        let response = accept::run(&mut ctx, envelope(vec![operation])).expect("accept");
        assert_eq!(response.results[0].disposition, Disposition::Accepted);
        response.results[0]
            .payload_digest
            .clone()
            .expect("the acceptance names a digest")
    };

    let stored = replica_read::revision(&peer.conn, "memory", &id)
        .expect("revision")
        .expect("row")
        .payload_digest
        .expect("digest");
    assert_eq!(
        stored, answered,
        "the revision must hold exactly the bytes the peer was told were accepted"
    );
    assert_eq!(
        crate::domains::memories::replica_payload::MemoryPayloadV1::from_record(
            &MemoryStore::new(peer.paths.clone())
                .load(&id)
                .expect("load")
        )
        .expect("payload")
        .canonical()
        .expect("canonical")
        .1,
        answered,
        "and the markdown on disk must re-derive that same digest"
    );
}
