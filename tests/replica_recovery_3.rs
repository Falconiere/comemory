#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! GC reach over real replica state (#256, B-6, AC-6): a real `comemory gc`
//! run against a real spawned engine, its journal, outbox and staged parts.

#[path = "common/replica_support.rs"]
mod replica_support;

#[path = "common/replica_events_support.rs"]
mod replica_events_support;

use comemory::store::replica_replay::{self, ReplayRow};
use comemory::store::replica_staging;
use comemory::store::sync_exchange::ExchangeKey;
use replica_events_support::{age, feedback, journal, pair};
use replica_support::{active_generation, feed_ops, mark_all_pushed, publish_locally};
use serde_json::json;

#[test]
fn gc_keeps_what_is_owed_and_expires_the_rest() {
    let p = pair();
    let active = publish_locally(&p.a.data_dir(), "checkout-a");

    // One verdict settles (an ordinary push acknowledgment) before the next
    // is recorded, so only its own outbox row is marked accepted; the second
    // stays pending — an upload this client still owes an unreachable hub.
    feedback(&p.a, Some("delivered"), &p.scene, false);
    let delivered_digest = journal(&p.a.data_dir(), "feedback_event")[0]["digest"]
        .as_str()
        .expect("digest")
        .to_string();
    mark_all_pushed(&p.a.data_dir());
    feedback(&p.a, Some("owed"), &p.scene, false);
    let entries = journal(&p.a.data_dir(), "feedback_event");
    assert_eq!(entries.len(), 2);
    let owed_entry = entries
        .iter()
        .find(|e| e["digest"] != json!(delivered_digest))
        .expect("the second event");
    let owed_digest = owed_entry["digest"].as_str().expect("digest").to_string();

    // Feedback verdicts are not themselves durably owed through the outbox
    // the way a memory upsert is — this stands in for the exchange gate
    // (#256, B-3) offering it once that lands, so the digest is genuinely
    // named by a `pending` row when retention runs.
    comemory::store::replica_outbox::enqueue(
        &p.a.db(),
        &comemory::store::replica_journal::NewOperation {
            operation_id: owed_entry["operation_id"].as_str().expect("operation_id"),
            entity_kind: "feedback_event",
            entity_key: owed_entry["entity_key"].as_str().expect("entity_key"),
            op: comemory::store::replica_journal::ReplicaOp::Upsert,
            payload: Some(comemory::store::replica_journal::PayloadRef {
                digest: &owed_digest,
                bytes: "unused: only the digest is read back",
            }),
            schema_version: 1,
            repository: Some("Falconiere/comemory"),
            origin: comemory::store::replica_journal::ReplicaOrigin::Local,
            at: "2026-09-25T10:00:00Z",
        },
        None,
    )
    .expect("enqueue the owed push");

    // A SIGKILLed pull's leftover scratch copy of the digest about to expire,
    // and a staged upload nobody ever finished — both must be gone once `gc`
    // reaches past retention, alongside the payload itself.
    let key = ExchangeKey::new("http://127.0.0.1:9/api", "ws_gc_test");
    replica_replay::offer(
        &p.a.db(),
        &key,
        &ReplayRow {
            entity_kind: "feedback_event".to_string(),
            entity_key: "scratch-leftover".to_string(),
            sequence: 1,
            entry_json: format!(r#"{{"payload_digest":"{delivered_digest}"}}"#),
        },
    )
    .expect("seed replay scratch");
    replica_staging::put_part(
        &p.a.db(),
        "stray-staging",
        0,
        1,
        "{}",
        "2020-01-01T00:00:00Z",
    )
    .expect("seed a stray staged part");

    age(&p.a.data_dir(), "feedback_events", "1 = 1");
    let feed_before = feed_ops(&p.a.data_dir());

    let response = p.a.cli(&["gc"]);
    assert_eq!(response["staged_rows"].as_u64(), Some(1), "{response}");

    let persisted_staged_rows: i64 =
        p.a.db()
            .query_row(
                "SELECT staged_rows FROM gc_runs ORDER BY at DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .expect("gc_runs row");
    assert_eq!(
        persisted_staged_rows, 1,
        "the sweep is persisted, not only reported"
    );

    let after = journal(&p.a.data_dir(), "feedback_event");
    let owed_after = after
        .iter()
        .find(|e| e["digest"] == json!(owed_digest))
        .expect("owed entry survives");
    assert!(
        owed_after["payload"].is_object(),
        "an owed upload keeps its bytes through gc: {owed_after}"
    );
    assert!(owed_after["redaction"].is_null());
    let delivered_after = after
        .iter()
        .find(|e| e["digest"] == json!(delivered_digest))
        .expect("delivered entry survives");
    assert!(
        delivered_after["payload"].is_null(),
        "a delivered, settled event expires: {delivered_after}"
    );
    assert_eq!(delivered_after["redaction"], json!("expired"));

    assert_eq!(
        replica_replay::next(&p.a.db(), &key, 10).expect("next"),
        Vec::<ReplayRow>::new(),
        "the killed replay's scratch copy of the expired digest is cleared"
    );
    let staged_left: i64 =
        p.a.db()
            .query_row("SELECT count(*) FROM replica_staged_part", [], |r| r.get(0))
            .expect("count");
    assert_eq!(staged_left, 0, "the abandoned upload is swept");

    assert_eq!(
        feed_ops(&p.a.data_dir()),
        feed_before,
        "no feed row is removed by either sweep"
    );
    assert_eq!(
        active_generation(&p.a.data_dir(), "checkout-a"),
        Some(active),
        "the active generation is untouched"
    );
}
