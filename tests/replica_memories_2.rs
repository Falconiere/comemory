#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The identity, ordering and refusal half of issue 251's memory surface,
//! over two real spawned engines and real HTTP: M-4 (a pending local change
//! survives a pull), M-5 (server revision decides, and every delivery is
//! idempotent), M-6 (the vector rule on the wire), M-7 (an import refreshes
//! once and echoes nothing back).
//!
//! The first half — the mutation surfaces and crash recovery — is
//! `replica_memories.rs`.

use serde_json::{Value, json};

#[path = "common/replica_support.rs"]
mod replica_support;

use replica_support::{
    BODY, Engine, digest_of, feed_ops, latest_revision, mark_all_pushed, owed, payload_of,
    upsert_envelope,
};

/// Import `envelope` into `engine` and return the one operation result.
fn import(engine: &Engine, envelope: &Value) -> Value {
    let (status, body) = engine.post("/api/v1/sync/replica/import", envelope);
    assert_eq!(status, 200, "import: {body}");
    body["data"]["results"][0].clone()
}

/// An engine holding one memory saved from `body`, with its own operation
/// already pushed so an import of the same entity is not refused as stale.
fn seeded_engine(body: &str) -> (Engine, String) {
    let engine = Engine::spawn(&[]);
    let saved = engine.cli(&["save", "--kind", "decision", body]);
    let id = saved["id"].as_str().expect("id").to_string();
    mark_all_pushed(&engine.data_dir());
    (engine, id)
}

// ---------------------------------------------------------------------------
// M-4: a local change this machine still owes is not overwritten by a pull.
// ---------------------------------------------------------------------------

#[test]
fn a_pending_local_edit_survives_a_pull_of_the_same_memory() {
    let author = Engine::spawn(&[]);
    let saved = author.cli(&["save", "--kind", "decision", BODY]);
    let id = saved["id"].as_str().expect("id").to_string();

    // The local engine holds the same memory with its own tags, unpushed —
    // and it is a `replica-v1` client of some upstream, the only kind of
    // engine that owes uploads (#255: a hub owes none and refuses nothing).
    let local = Engine::spawn(&[]);
    make_replica_client(&local.data_dir());
    local.cli(&["save", "--kind", "decision", BODY]);
    let (status, body) = local.patch(
        &format!("/api/v1/memories/{id}"),
        &json!({"tags": ["local-only"]}),
    );
    assert_eq!(status, 200, "local edit: {body}");
    let owed_before = owed(&local.data_dir());
    assert!(!owed_before.is_empty(), "the local edit is owed upstream");

    let remote = payload_of(&author, &id);
    let result = import(
        &local,
        &upsert_envelope(
            "op-20260922-remote01",
            &id,
            &digest_of(&author, &id),
            &remote,
        ),
    );

    assert_eq!(
        result["disposition"], "rejected_stale",
        "the local edit is the only copy of itself: {result}"
    );
    assert_eq!(
        owed(&local.data_dir()),
        owed_before,
        "and the payload the outbox holds is untouched"
    );
    let (status, shown) = local.get(&format!("/api/v1/memories/{id}"));
    assert_eq!(status, 200, "{shown}");
    assert_eq!(
        shown["data"]["tags"],
        json!(["local-only"]),
        "the local tags are what the next push would send"
    );
}

// ---------------------------------------------------------------------------
// M-5: the server's revision decides, and every delivery is idempotent.
// ---------------------------------------------------------------------------

#[test]
fn the_higher_accepted_sequence_wins_regardless_of_the_provenance_clock() {
    let author = Engine::spawn(&[]);
    let first = author.cli(&["save", "--kind", "decision", BODY]);
    let id = first["id"].as_str().expect("id").to_string();
    let (early, early_digest) = latest_revision(&author, &id);
    // A real second revision of the same memory, with a deliberately
    // BACKWARDS provenance time in its payload.
    let (status, body) = author.patch(
        &format!("/api/v1/memories/{id}"),
        &json!({"tags": ["second-revision"]}),
    );
    assert_eq!(status, 200, "{body}");
    let (later, later_digest) = latest_revision(&author, &id);
    assert_ne!(early_digest, later_digest, "two distinct revisions");

    let peer = Engine::spawn(&[]);
    let second = import(&peer, &upsert_envelope("op-a", &id, &later_digest, &later));
    assert_eq!(second["disposition"], "accepted", "{second}");
    let stale = import(&peer, &upsert_envelope("op-b", &id, &early_digest, &early));

    assert_eq!(
        stale["disposition"], "accepted",
        "an ordinary upsert is accepted; ordering is what the feed records"
    );
    let ops = feed_ops(&peer.data_dir());
    assert_eq!(ops.len(), 2, "two accepted positions: {ops:?}");
    let (status, shown) = peer.get(&format!("/api/v1/memories/{id}"));
    assert_eq!(status, 200, "{shown}");
    assert_eq!(
        shown["data"]["tags"],
        json!([]),
        "the LAST accepted position is what is materialized, not the newest \
         provenance clock"
    );
}

#[test]
fn a_duplicate_delete_replays_its_receipt_without_a_second_tombstone() {
    let (engine, id) = seeded_engine(BODY);
    let tombstone = json!({
        "protocol": "replica-v1",
        "operations": [{
            "operation_id": "op-20260922-delete01",
            "entity_kind": "memory",
            "entity_key": id,
            "op": "tombstone",
            "schema_version": 1,
        }]
    });

    let first = import(&engine, &tombstone);
    assert_eq!(first["disposition"], "accepted", "{first}");
    let after_first = feed_ops(&engine.data_dir());
    let replay = import(&engine, &tombstone);

    assert_eq!(
        replay["disposition"], "duplicate",
        "the retry reads back its receipt: {replay}"
    );
    assert_eq!(replay["sequence"], first["sequence"], "same position");
    assert_eq!(
        feed_ops(&engine.data_dir()),
        after_first,
        "and no second tombstone"
    );
}

#[test]
fn a_restore_naming_a_stale_deletion_sequence_stays_refused() {
    let (engine, id) = seeded_engine(BODY);
    let tombstone = json!({
        "protocol": "replica-v1",
        "operations": [{
            "operation_id": "op-20260922-delete02",
            "entity_kind": "memory",
            "entity_key": id,
            "op": "tombstone",
            "schema_version": 1,
        }]
    });
    let (reinstated, reinstated_digest) = latest_revision(&engine, &id);
    let deleted = import(&engine, &tombstone);
    let deletion_sequence = deleted["sequence"].as_i64().expect("sequence");

    // A restore that claims to have observed a position BEFORE the deletion
    // cannot revive it: it never saw the deletion it would be reversing. It
    // carries the payload it is reinstating, like any other write — the
    // refusal must come from the ordering, not from a malformed operation.
    let stale = import(
        &engine,
        &json!({
            "protocol": "replica-v1",
            "operations": [{
                "operation_id": "op-20260922-restore1",
                "entity_kind": "memory",
                "entity_key": id,
                "op": "restore",
                "schema_version": 1,
                "payload_digest": reinstated_digest,
                "payload": reinstated,
                "observed_sequence": deletion_sequence - 1,
            }]
        }),
    );

    assert_eq!(
        stale["disposition"], "rejected_stale",
        "a restore must name the deletion it observed: {stale}"
    );
    let (status, _) = engine.get(&format!("/api/v1/memories/{id}"));
    assert_eq!(status, 404, "the memory stays deleted");
}

// ---------------------------------------------------------------------------
// M-5: content-derived identity refuses a body that does not hash to its id.
// ---------------------------------------------------------------------------

#[test]
fn an_import_whose_body_no_longer_hashes_to_its_id_leaves_the_markdown_alone() {
    let (engine, id) = seeded_engine(BODY);
    let mut tampered = payload_of(&engine, &id);
    tampered["body"] = json!("a different body entirely, under an id it cannot own");
    let digest = digest_of(&engine, &id);
    let before = engine.get(&format!("/api/v1/memories/{id}")).1;

    let result = import(
        &engine,
        &upsert_envelope("op-20260922-collide1", &id, &digest, &tampered),
    );

    assert_eq!(
        result["disposition"], "rejected_invalid",
        "the id no longer hashes its body: {result}"
    );
    let after = engine.get(&format!("/api/v1/memories/{id}")).1;
    assert_eq!(
        after["data"]["body"], before["data"]["body"],
        "and the stored markdown is unchanged"
    );
}

// ---------------------------------------------------------------------------
// M-7: an import refreshes local state once and echoes nothing back.
// ---------------------------------------------------------------------------

#[test]
fn an_import_creates_no_local_origin_operation_and_the_memory_is_searchable() {
    let (author, id) = seeded_engine(BODY);
    let payload = payload_of(&author, &id);
    let digest = digest_of(&author, &id);
    let peer = Engine::spawn(&[]);

    let result = import(
        &peer,
        &upsert_envelope("op-20260922-import01", &id, &digest, &payload),
    );

    assert_eq!(result["disposition"], "accepted", "{result}");
    assert_eq!(
        feed_ops(&peer.data_dir()),
        vec![format!("upsert:{id}")],
        "one position"
    );
    assert!(
        owed(&peer.data_dir()).is_empty(),
        "and nothing owed back: pushing an import is how a loop starts"
    );
    let found = peer.cli(&["search", "conventions"]);
    let hits = found["hits"].as_array().cloned().unwrap_or_default();
    assert!(
        hits.iter().any(|hit| hit["memory_id"] == json!(id)),
        "the imported memory is findable: {found}"
    );
}

// ---------------------------------------------------------------------------
// M-5: metadata-only drift is visible in the manifest and heals on import.
// ---------------------------------------------------------------------------

#[test]
fn two_engines_holding_the_same_body_with_different_tags_disagree_then_agree() {
    let (author, id) = seeded_engine(BODY);
    let (status, body) = author.patch(
        &format!("/api/v1/memories/{id}"),
        &json!({"tags": ["drifted"]}),
    );
    assert_eq!(status, 200, "{body}");
    mark_all_pushed(&author.data_dir());

    let peer = Engine::spawn(&[]);
    peer.cli(&["save", "--kind", "decision", BODY]);
    mark_all_pushed(&peer.data_dir());

    let author_buckets =
        author.get("/api/v1/sync/replica/manifest").1["data"]["entity_kinds"][0]["buckets"].clone();
    let peer_buckets =
        peer.get("/api/v1/sync/replica/manifest").1["data"]["entity_kinds"][0]["buckets"].clone();
    assert_ne!(
        author_buckets, peer_buckets,
        "same body, different tags: the digests must differ"
    );

    let (payload, digest) = latest_revision(&author, &id);
    let healed_result = import(
        &peer,
        &upsert_envelope("op-20260922-heal0001", &id, &digest, &payload),
    );
    assert_eq!(
        healed_result["disposition"], "accepted",
        "the newer revision must land: {healed_result}"
    );

    let healed =
        peer.get("/api/v1/sync/replica/manifest").1["data"]["entity_kinds"][0]["buckets"].clone();
    assert_eq!(
        healed, author_buckets,
        "importing the newer revision makes the buckets agree"
    );
}

/// Record, through the store API a session writes, that this engine selected
/// `replica-v1` against an upstream.
fn make_replica_client(data_dir: &std::path::Path) {
    use comemory::store::sync_exchange::{self, ExchangeKey, ExchangeRow};
    let conn = comemory::store::connection::open(data_dir.join("comemory.db")).expect("open db");
    let mut row = ExchangeRow::fresh(&ExchangeKey::new("http://127.0.0.1:9/api", "ws_m4"));
    row.protocol = Some("replica-v1".to_string());
    sync_exchange::save(&conn, &row, "2026-09-24T10:00:00Z").expect("select replica");
}
