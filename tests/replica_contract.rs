#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The `replica-v1` contract over the real surface: a real `comemory save`,
//! a real spawned `comemory serve`, real HTTP, and the real SQLite database
//! both sides keep — digests, replays, epochs, caps and staging.
//!
//! Nothing here is simulated. The peer engine is a second data directory
//! served by its own process, and every payload comes from a memory the CLI
//! actually saved. The fixture lives in `tests/common/replica_support.rs`, and
//! the second half of the contract is `replica_contract_2.rs`.

use serde_json::{Value, json};

#[path = "common/replica_support.rs"]
mod replica_support;

use replica_support::{BODY, Engine, digest_of, payload_of, upsert_envelope};

#[test]
fn payload_digest_covers_metadata_and_history_keeps_the_bytes_it_accepted() {
    let engine = Engine::spawn(&[]);
    let saved = engine.cli(&["save", BODY, "--kind", "decision", "--tags", "sync"]);
    let id = saved["id"].as_str().expect("id").to_string();
    let first_digest = digest_of(&engine, &id);
    let first_payload = payload_of(&engine, &id);

    // A tags-only edit through the real HTTP surface.
    let (status, patched) = engine.post(
        &format!("/api/v1/memories/{id}"),
        &json!({"tags": ["sync", "journal"]}),
    );
    assert!(
        status == 200 || status == 405,
        "unexpected patch status {status}: {patched}"
    );
    if status == 405 {
        // The console patches with PATCH; fall back to it.
        let response = reqwest::blocking::Client::new()
            .patch(format!("{}/api/v1/memories/{id}", engine.base))
            .bearer_auth(&engine.token)
            .json(&json!({"tags": ["sync", "journal"]}))
            .send()
            .expect("PATCH");
        assert_eq!(response.status().as_u16(), 200);
    }

    let second_digest = digest_of(&engine, &id);
    assert_ne!(
        first_digest, second_digest,
        "a metadata-only edit is a new revision"
    );

    let (_, body) = engine.get("/api/v1/sync/replica/changes?since=0&limit=50");
    let entries = body["data"]["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 2, "one save and one edit: {body}");
    assert_eq!(
        entries[0]["payload"], first_payload,
        "the first position still carries the bytes accepted at it"
    );

    let stored_body_hash: String = engine
        .db()
        .query_row(
            "SELECT content_hash FROM memories WHERE id = ?1",
            [&id],
            |r| r.get(0),
        )
        .expect("content hash");
    assert_ne!(
        stored_body_hash, second_digest,
        "content_hash hashes the body; the payload digest covers the metadata too"
    );
}

#[test]
fn replay_receipt_returns_the_original_position_and_conflicting_bytes_are_refused() {
    let author = Engine::spawn(&[]);
    let peer = Engine::spawn(&[]);
    let saved = author.cli(&["save", BODY, "--kind", "decision", "--tags", "sync"]);
    let id = saved["id"].as_str().expect("id").to_string();
    let digest = digest_of(&author, &id);
    let payload = payload_of(&author, &id);

    let envelope = upsert_envelope("op-20260921-aaaaaaaa", &id, &digest, &payload);
    let (status, first) = peer.post("/api/v1/sync/replica/import", &envelope);
    assert_eq!(status, 200, "import: {first}");
    let first_result = &first["data"]["results"][0];
    assert_eq!(first_result["disposition"], json!("accepted"));
    let sequence = first_result["sequence"].as_i64().expect("sequence");

    let (status, replay) = peer.post("/api/v1/sync/replica/import", &envelope);
    assert_eq!(status, 200, "replay: {replay}");
    assert_eq!(
        replay["data"]["results"][0]["disposition"],
        json!("duplicate")
    );
    assert_eq!(
        replay["data"]["results"][0]["sequence"].as_i64(),
        Some(sequence),
        "a replay never receives a newer position"
    );

    let mut tampered = envelope.clone();
    tampered["operations"][0]["payload"]["body"] = json!("a different body entirely");
    let (status, conflict) = peer.post("/api/v1/sync/replica/import", &tampered);
    assert_eq!(status, 200, "conflict: {conflict}");
    assert_eq!(
        conflict["data"]["results"][0]["disposition"],
        json!("rejected_conflict")
    );

    let positions: i64 = peer
        .db()
        .query_row("SELECT count(*) FROM replica_feed", [], |r| r.get(0))
        .expect("count");
    assert_eq!(positions, 1, "only the first acceptance wrote a position");
    let stored: String = peer
        .db()
        .query_row("SELECT body FROM memories WHERE id = ?1", [&id], |r| {
            r.get(0)
        })
        .expect("body");
    assert!(stored.contains("durable, searchable memory"));
}

#[test]
fn epoch_mismatch_is_reported_rather_than_answered_with_an_empty_page() {
    let engine = Engine::spawn(&[]);
    engine.cli(&["save", BODY, "--kind", "decision"]);

    let (status, body) = engine.get(&format!(
        "/api/v1/sync/replica/changes?since=0&epoch={}",
        "0".repeat(32)
    ));
    assert_eq!(status, 409, "a foreign epoch is a conflict: {body}");
    assert_eq!(body["error"]["code"], json!("epoch_mismatch"));

    let epoch: String = engine
        .db()
        .query_row("SELECT epoch FROM replica_stream", [], |r| r.get(0))
        .expect("epoch");
    // At the head: caught up, with the real head echoed back.
    let (status, caught_up) = engine.get(&format!(
        "/api/v1/sync/replica/changes?since=1&epoch={epoch}"
    ));
    assert_eq!(status, 200, "{caught_up}");
    assert_eq!(caught_up["data"]["entries"], json!([]));
    assert_eq!(caught_up["data"]["head_sequence"], json!(1));
    assert_eq!(caught_up["data"]["next_sequence"], Value::Null);

    // Above the head: a position this stream never issued, refused rather
    // than answered with an empty page the peer would read as agreement.
    let (status, ahead) = engine.get(&format!(
        "/api/v1/sync/replica/changes?since=99&epoch={epoch}"
    ));
    assert_eq!(status, 409, "a cursor past the head is a conflict: {ahead}");
    assert_eq!(ahead["error"]["code"], json!("conflict"));
}

#[test]
fn envelope_caps_refuse_an_oversized_batch_without_applying_anything() {
    let author = Engine::spawn(&[]);
    let peer = Engine::spawn(&[]);
    let saved = author.cli(&["save", BODY, "--kind", "decision"]);
    let id = saved["id"].as_str().expect("id").to_string();
    let digest = digest_of(&author, &id);
    let payload = payload_of(&author, &id);

    let mut operations = Vec::new();
    for n in 0..501 {
        operations.push(json!({
            "operation_id": format!("op-20260921-{n:08x}"),
            "entity_kind": "memory",
            "entity_key": id,
            "op": "upsert",
            "schema_version": 1,
            "payload_digest": digest,
            "payload": payload,
        }));
    }
    let (status, body) = peer.post(
        "/api/v1/sync/replica/import",
        &json!({"protocol": "replica-v1", "operations": operations}),
    );
    assert_eq!(status, 400, "501 operations is over the cap: {body}");

    let positions: i64 = peer
        .db()
        .query_row("SELECT count(*) FROM replica_feed", [], |r| r.get(0))
        .expect("count");
    assert_eq!(positions, 0, "a refused envelope applies nothing");

    // Above the 5 MiB byte cap: refused before the body is accepted, which a
    // client sees either as a status or as a closed connection.
    let mut oversized = payload.clone();
    oversized["body"] = json!("x".repeat(6 * 1024 * 1024));
    let status = peer.post_expecting_refusal(
        "/api/v1/sync/replica/import",
        &json!({
            "protocol": "replica-v1",
            "operations": [{
                "operation_id": "op-20260921-toolarge",
                "entity_kind": "memory",
                "entity_key": id,
                "op": "upsert",
                "schema_version": 1,
                "payload_digest": digest,
                "payload": oversized,
            }]
        }),
    );
    if let Some(status) = status {
        assert!(
            status == 413 || status == 400,
            "a >5 MiB envelope must be refused, got {status}"
        );
    }
    let after: i64 = peer
        .db()
        .query_row("SELECT count(*) FROM replica_feed", [], |r| r.get(0))
        .expect("count");
    assert_eq!(after, 0, "an oversized envelope applies nothing either");
}

#[test]
fn staged_parts_are_invisible_until_activation_completes_them() {
    let author = Engine::spawn(&[]);
    let peer = Engine::spawn(&[]);
    let saved = author.cli(&["save", BODY, "--kind", "decision"]);
    let id = saved["id"].as_str().expect("id").to_string();
    let digest = digest_of(&author, &id);
    let payload = payload_of(&author, &id);
    let bytes = serde_json::to_string(&payload).expect("payload bytes");
    let mut mid = bytes.len() / 2;
    while !bytes.is_char_boundary(mid) {
        mid += 1;
    }

    let (status, staged) = peer.post(
        "/api/v1/sync/replica/stage",
        &json!({
            "protocol": "replica-v1",
            "staging_id": "stage-1",
            "part_index": 0,
            "part_count": 2,
            "bytes": &bytes[..mid],
        }),
    );
    assert_eq!(status, 200, "stage: {staged}");
    assert_eq!(staged["data"]["complete"], json!(false));

    let activation = json!({
        "protocol": "replica-v1",
        "staging_id": "stage-1",
        "operation": {
            "operation_id": "op-20260921-bbbbbbbb",
            "entity_kind": "memory",
            "entity_key": id,
            "op": "upsert",
            "schema_version": 1,
            "payload_digest": digest,
        }
    });
    let (status, incomplete) = peer.post("/api/v1/sync/replica/activate", &activation);
    assert_eq!(
        status, 409,
        "a missing part refuses activation: {incomplete}"
    );

    let (_, page) = peer.get("/api/v1/sync/replica/changes?since=0");
    assert_eq!(
        page["data"]["entries"],
        json!([]),
        "a half-uploaded revision is not history"
    );

    peer.post(
        "/api/v1/sync/replica/stage",
        &json!({
            "protocol": "replica-v1",
            "staging_id": "stage-1",
            "part_index": 1,
            "part_count": 2,
            "bytes": &bytes[mid..],
        }),
    );
    let (status, activated) = peer.post("/api/v1/sync/replica/activate", &activation);
    assert_eq!(status, 200, "activate: {activated}");
    assert_eq!(activated["data"]["disposition"], json!("accepted"));

    let (_, page) = peer.get("/api/v1/sync/replica/changes?since=0");
    assert_eq!(
        page["data"]["entries"].as_array().expect("entries").len(),
        1
    );
}
