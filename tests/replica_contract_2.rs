#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The `replica-v1` contract, second half: tombstone ordering, workspace
//! scope, route classes, daemon independence and legacy-wire convergence.
//!
//! Same real-process fixture as `replica_contract.rs`.

use serde_json::json;

#[path = "common/replica_support.rs"]
mod replica_support;

use replica_support::{BODY, Engine, digest_of, payload_of, upsert_envelope};

#[test]
fn tombstone_order_holds_against_a_stale_edit_and_an_informed_restore() {
    let author = Engine::spawn(&[]);
    let peer = Engine::spawn(&[]);
    let saved = author.cli(&["save", BODY, "--kind", "decision"]);
    let id = saved["id"].as_str().expect("id").to_string();
    let digest = digest_of(&author, &id);
    let payload = payload_of(&author, &id);

    peer.post(
        "/api/v1/sync/replica/import",
        &upsert_envelope("op-1", &id, &digest, &payload),
    );
    let (_, deleted) = peer.post(
        "/api/v1/sync/replica/import",
        &json!({
            "protocol": "replica-v1",
            "operations": [{
                "operation_id": "op-2",
                "entity_kind": "memory",
                "entity_key": id,
                "op": "tombstone",
                "schema_version": 1,
            }]
        }),
    );
    let deletion_sequence = deleted["data"]["results"][0]["sequence"]
        .as_i64()
        .expect("sequence");

    let (_, stale) = peer.post(
        "/api/v1/sync/replica/import",
        &upsert_envelope("op-3", &id, &digest, &payload),
    );
    assert_eq!(
        stale["data"]["results"][0]["disposition"],
        json!("rejected_stale"),
        "an upsert that never saw the deletion cannot revive it"
    );

    let (_, restored) = peer.post(
        "/api/v1/sync/replica/import",
        &json!({
            "protocol": "replica-v1",
            "operations": [{
                "operation_id": "op-4",
                "entity_kind": "memory",
                "entity_key": id,
                "op": "restore",
                "schema_version": 1,
                "payload_digest": digest,
                "payload": payload,
                "observed_sequence": deletion_sequence,
            }]
        }),
    );
    assert_eq!(
        restored["data"]["results"][0]["disposition"],
        json!("accepted")
    );
    assert!(
        restored["data"]["results"][0]["sequence"]
            .as_i64()
            .expect("sequence")
            > deletion_sequence,
        "a restore lands above the tombstone it reverses"
    );
}

#[test]
fn workspace_scope_comes_from_the_credential_not_the_body() {
    let author = Engine::spawn(&[]);
    let peer = Engine::spawn(&[]);
    let saved = author.cli(&["save", BODY, "--kind", "decision"]);
    let id = saved["id"].as_str().expect("id").to_string();
    let digest = digest_of(&author, &id);
    let payload = payload_of(&author, &id);

    let mut envelope = upsert_envelope("op-1", &id, &digest, &payload);
    envelope["workspace_id"] = json!("ws_someone_elses");
    let (status, body) = peer.post("/api/v1/sync/replica/import", &envelope);
    assert_eq!(status, 400, "a body cannot claim a workspace: {body}");

    let cursors: i64 = peer
        .db()
        .query_row("SELECT count(*) FROM replica_cursor", [], |r| r.get(0))
        .expect("count");
    assert_eq!(cursors, 0, "the claimed workspace never reached a cursor");
    let positions: i64 = peer
        .db()
        .query_row("SELECT count(*) FROM replica_feed", [], |r| r.get(0))
        .expect("count");
    assert_eq!(positions, 0);
}

#[test]
fn route_classes_hold_against_a_read_only_engine() {
    let engine = Engine::spawn(&["--read-only"]);

    for path in [
        "/api/v1/sync/replica/changes?since=0",
        "/api/v1/sync/replica/manifest",
        "/api/v1/sync/replica/events?since=0",
    ] {
        let (status, body) = engine.get(path);
        assert_eq!(status, 200, "{path} is read-class: {body}");
    }

    for (path, body) in [
        (
            "/api/v1/sync/replica/import",
            json!({"protocol": "replica-v1", "operations": []}),
        ),
        (
            "/api/v1/sync/replica/stage",
            json!({"protocol":"replica-v1","staging_id":"s","part_index":0,"part_count":1,"bytes":"{}"}),
        ),
    ] {
        let (status, refused) = engine.post(path, &body);
        assert!(
            status == 403 || status == 405 || status == 409,
            "{path} must be refused in read-only mode, got {status}: {refused}"
        );
    }
}

#[test]
fn no_replica_request_starts_a_daemon() {
    let engine = Engine::spawn(&[]);
    engine.cli(&["save", BODY, "--kind", "decision"]);
    for path in [
        "/api/v1/sync/replica/changes?since=0",
        "/api/v1/sync/replica/manifest",
        "/api/v1/sync/replica/events?since=0",
    ] {
        let (status, _) = engine.get(path);
        assert_eq!(status, 200);
    }

    let data_dir = engine.data_dir();
    for leftover in ["daemon.pid", "daemon.log"] {
        assert!(
            !data_dir.join(leftover).exists(),
            "the request path must not start a daemon ({leftover})"
        );
    }
    let units = std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
        .join("Library/LaunchAgents/io.comemory.daemon.plist");
    assert!(
        !units.exists() || !data_dir.join("auth.json").exists(),
        "no unit was installed by a replica request"
    );
}

#[test]
fn legacy_sync_routes_keep_their_shape_and_still_write_the_journal() {
    let author = Engine::spawn(&[]);
    let peer = Engine::spawn(&[]);
    let saved = author.cli(&["save", BODY, "--kind", "decision", "--tags", "sync"]);
    let id = saved["id"].as_str().expect("id").to_string();

    let (status, legacy) = author.get("/api/v1/sync/changes?since=0&limit=10");
    assert_eq!(status, 200, "legacy changes: {legacy}");
    let entry = &legacy["data"]["entries"][0];
    assert_eq!(entry["id"], json!(id));
    assert!(entry["record"]["frontmatter"]["content_hash"].is_string());
    assert!(entry["seq"].is_i64(), "the legacy wire keeps `seq`");

    let (status, imported) = peer.post(
        "/api/v1/sync/import",
        &json!({
            "cursor": 0,
            "entries": [{
                "op": entry["op"],
                "id": entry["id"],
                "content_hash": entry["content_hash"],
                "at": entry["at"],
                "record": entry["record"],
            }]
        }),
    );
    assert_eq!(status, 200, "legacy import: {imported}");
    assert_eq!(imported["data"]["results"][0]["status"], json!("accepted"));

    let positions: i64 = peer
        .db()
        .query_row("SELECT count(*) FROM replica_feed", [], |r| r.get(0))
        .expect("count");
    assert_eq!(
        positions, 1,
        "a legacy import writes the replica journal too, so both wires converge"
    );
}

#[test]
fn a_rebuild_preserves_sequences_receipts_and_the_stream_epoch() {
    let author = Engine::spawn(&[]);
    let peer = Engine::spawn(&[]);
    let saved = author.cli(&["save", BODY, "--kind", "decision"]);
    let id = saved["id"].as_str().expect("id").to_string();
    let digest = digest_of(&author, &id);
    let payload = payload_of(&author, &id);

    let (status, accepted) = peer.post(
        "/api/v1/sync/replica/import",
        &upsert_envelope("op-20260921-cccccccc", &id, &digest, &payload),
    );
    assert_eq!(status, 200, "import: {accepted}");
    let sequence = accepted["data"]["results"][0]["sequence"]
        .as_i64()
        .expect("sequence");
    let epoch: String = peer
        .db()
        .query_row("SELECT epoch FROM replica_stream", [], |r| r.get(0))
        .expect("epoch");

    // Stop the server so the rebuild owns the database, then rebuild from
    // markdown exactly as an operator would.
    let rebuilt = Engine::reopen(peer);
    rebuilt.cli(&["rebuild"]);

    let db = rebuilt.db();
    let preserved_epoch: String = db
        .query_row("SELECT epoch FROM replica_stream", [], |r| r.get(0))
        .expect("epoch");
    assert_eq!(preserved_epoch, epoch, "a rebuild is not a replaced stream");

    let (kept_sequence, kept_disposition): (i64, String) = db
        .query_row(
            "SELECT sequence, disposition FROM replica_receipt WHERE operation_id = ?1",
            ["op-20260921-cccccccc"],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("receipt survived");
    assert_eq!(kept_sequence, sequence);
    assert_eq!(kept_disposition, "accepted");

    let feed_head: i64 = db
        .query_row("SELECT MAX(sequence) FROM replica_feed", [], |r| r.get(0))
        .expect("head");
    assert_eq!(feed_head, sequence, "positions survived the rebuild");

    let next: i64 = db
        .query_row(
            "SELECT seq FROM sqlite_sequence WHERE name = 'replica_feed'",
            [],
            |r| r.get(0),
        )
        .expect("autoincrement high-water mark");
    assert!(
        next >= sequence,
        "the next acceptance must land above the preserved head, not reuse {sequence}"
    );
}
