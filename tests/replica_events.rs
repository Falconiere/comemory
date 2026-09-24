#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Feedback verdicts over the real surface (#254): the real CLI and HTTP
//! surfaces producing real verdicts on real searches, spawned `comemory serve`
//! engines, and the `replica-v1` routes carrying them between machines.
//!
//! This file covers identity and provenance (AC-1, AC-3), exactly-once
//! counting under replay and echo (AC-4), and atomicity under an induced fault
//! and a kill (AC-5). Backfill, ground truth, purge, rebuild and an evicted
//! query row are `replica_events_3.rs`; activity is `replica_events_2.rs`.
//! `bash scripts/test-replication-e2e.sh --case events` runs all three.

#[path = "common/replica_support.rs"]
mod replica_support;

#[path = "common/replica_events_support.rs"]
mod replica_events_support;

use std::time::Duration;

use replica_events_support::{
    CANONICAL, SOURCE_FILE, SYMBOL, approve, changes, code_counters, count, device_of,
    dispositions, feedback, feedback_rows, import, journal, memory_counters, operation_of, pair,
    transfer,
};
use replica_support::Engine;
use serde_json::{Value, json};

const KIND: &str = "feedback_event";

fn is_event_id(value: &Value) -> bool {
    value.as_str().is_some_and(|s| {
        s.len() == 35
            && s.starts_with("ev-")
            && s[3..]
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    })
}

fn row_for<'a>(rows: &'a [Value], event_id: &Value) -> &'a Value {
    rows.iter()
        .find(|r| &r["event_id"] == event_id)
        .unwrap_or_else(|| panic!("no row for {event_id}: {rows:?}"))
}

#[test]
fn a_verdict_travels_with_its_provenance_and_a_canonical_target() {
    let p = pair();
    feedback(&p.a, Some("replica-test/1.0"), &p.scene, true);

    let rows = feedback_rows(&p.a.data_dir());
    assert_eq!(rows.len(), 2, "one memory and one code verdict: {rows:?}");
    for row in &rows {
        assert!(is_event_id(&row["event_id"]), "stable event id: {row}");
        assert_eq!(row["surface"], json!("cli"));
        assert_eq!(row["actor"], json!("replica-test/1.0"));
        assert_eq!(row["device"], Value::Null, "recorded here");
    }
    let device = device_of(&p.a.data_dir());
    assert_eq!(device.len(), 32, "a 32-hex device id");
    assert_ne!(
        device,
        device_of(&p.b.data_dir()),
        "each database has its own"
    );

    let blob: String =
        p.a.db()
            .query_row(
                "SELECT blob_oid FROM code_symbols WHERE id = ?1",
                [p.scene.symbol],
                |r| r.get(0),
            )
            .expect("blob oid");
    let journalled = journal(&p.a.data_dir(), KIND);
    assert_eq!(journalled.len(), 2, "one position per verdict");
    for entry in &journalled {
        let payload = &entry["payload"];
        let row = row_for(&rows, &payload["event_id"]);
        assert_eq!(entry["entity_key"], payload["event_id"]);
        assert_eq!(entry["origin"], json!("local"));
        assert_eq!(entry["repository"], json!(CANONICAL));
        assert_eq!(payload["device"], json!(device));
        assert_eq!(payload["at"], row["at"], "the original timestamp");
        assert_eq!(payload["surface"], json!("cli"));
        assert_eq!(payload["actor"], json!("replica-test/1.0"));
        assert_eq!(payload["provenance"], json!("manual"));
        assert_eq!(payload["verdict"], json!("used"));
        assert_eq!(
            payload["origin_query_id"],
            json!(format!("{device}:{}", p.scene.query_id)),
            "the query id is namespaced by the device that asked"
        );
        match payload["target"]["kind"].as_str() {
            Some("memory") => assert_eq!(
                payload["target"],
                json!({"kind": "memory", "id": p.scene.memory})
            ),
            Some("code") => assert_eq!(
                payload["target"],
                json!({"kind": "code", "repo": CANONICAL, "path": SOURCE_FILE,
                       "symbol": SYMBOL, "version": blob})
            ),
            other => panic!("unexpected target {other:?}: {payload}"),
        }
    }

    let results = transfer(&p.a, &p.b, &[KIND]);
    assert_eq!(dispositions(&results), vec!["accepted", "accepted"]);
    let imported = feedback_rows(&p.b.data_dir());
    assert_eq!(imported.len(), 2);
    for row in &imported {
        let local = row_for(&rows, &row["event_id"]);
        assert_eq!(
            row["device"],
            json!(device),
            "attributed to its origin, not the receiver"
        );
        assert_eq!(row["at"], local["at"]);
        assert_eq!(row["surface"], local["surface"]);
        assert_eq!(row["actor"], local["actor"]);
        assert_eq!(row["provenance"], json!("manual"));
        assert_eq!(
            row["query_id"],
            json!(format!("{device}:{}", p.scene.query_id))
        );
    }
    assert_eq!(
        count(&p.b.data_dir(), "retrieval_log"),
        0,
        "no query row is invented on the receiver"
    );
    assert_eq!(
        memory_counters(&p.b.data_dir()),
        vec![(p.scene.memory.clone(), 1, 0)]
    );
    assert_eq!(
        code_counters(&p.b.data_dir()),
        vec![(
            "checkout-b".to_string(),
            SOURCE_FILE.to_string(),
            SYMBOL.to_string(),
            1,
            0
        )],
        "keyed by the receiver's own label for the canonical repository"
    );

    let unapproved = Engine::spawn(&[]);
    transfer(&p.a, &unapproved, &[KIND]);
    assert_eq!(
        code_counters(&unapproved.data_dir()),
        vec![(
            CANONICAL.to_string(),
            SOURCE_FILE.to_string(),
            SYMBOL.to_string(),
            1,
            0
        )],
        "with no approved label the canonical name is the key"
    );
}

#[test]
fn two_verdicts_count_twice_while_replays_and_echoes_count_nothing() {
    let p = pair();
    feedback(&p.a, None, &p.scene, true);
    feedback(&p.a, None, &p.scene, true);

    let first = transfer(&p.a, &p.b, &[KIND]);
    assert_eq!(dispositions(&first), vec!["accepted"; 4]);
    let counted =
        |data_dir: &std::path::Path| (memory_counters(data_dir)[0].1, code_counters(data_dir)[0].3);
    assert_eq!(
        counted(&p.b.data_dir()),
        (2, 2),
        "two real calls are two verdicts"
    );

    for _ in 0..2 {
        let replay = transfer(&p.a, &p.b, &[KIND]);
        assert_eq!(dispositions(&replay), vec!["duplicate"; 4]);
        let sequences: Vec<&Value> = replay.iter().map(|r| &r["sequence"]).collect();
        let original: Vec<&Value> = first.iter().map(|r| &r["sequence"]).collect();
        assert_eq!(sequences, original, "a replay reads its original position");
    }
    assert_eq!(counted(&p.b.data_dir()), (2, 2));

    let echo = transfer(&p.b, &p.a, &[KIND]);
    assert_eq!(
        dispositions(&echo),
        vec!["duplicate"; 4],
        "the sender recognizes its own events under their own operation ids"
    );
    assert_eq!(
        counted(&p.a.data_dir()),
        (2, 2),
        "the echo moved no counter"
    );
    assert_eq!(counted(&p.b.data_dir()), (2, 2));
}

#[test]
fn a_failed_counter_write_leaves_nothing_and_the_retry_applies_once() {
    let p = pair();
    feedback(&p.a, None, &p.scene, false);
    let operations: Vec<Value> = changes(&p.a, &[KIND]).iter().map(operation_of).collect();
    assert_eq!(operations.len(), 1);

    p.b.db()
        .execute_batch(
            "CREATE TRIGGER induced_failure BEFORE INSERT ON feedback \
             BEGIN SELECT RAISE(ABORT, 'induced counter failure'); END;",
        )
        .expect("install trigger");
    let (status, body) = p.b.post(
        "/api/v1/sync/replica/import",
        &json!({"protocol": "replica-v1", "operations": operations}),
    );
    assert_ne!(status, 200, "the induced failure surfaces: {body}");
    assert_eq!(count(&p.b.data_dir(), "feedback_events"), 0);
    assert_eq!(count(&p.b.data_dir(), "replica_receipt"), 0);
    assert!(
        journal(&p.b.data_dir(), KIND).is_empty(),
        "no position either"
    );

    p.b.db()
        .execute_batch("DROP TRIGGER induced_failure;")
        .expect("drop trigger");
    assert_eq!(dispositions(&import(&p.b, &operations)), vec!["accepted"]);
    assert_eq!(dispositions(&import(&p.b, &operations)), vec!["duplicate"]);
    assert_eq!(
        memory_counters(&p.b.data_dir()),
        vec![(p.scene.memory.clone(), 1, 0)]
    );
}

#[test]
fn a_killed_import_leaves_every_event_whole() {
    let workspace = tempfile::tempdir().expect("workspace");
    let a = Engine::spawn(&[]);
    approve(&a, &[("checkout-a", CANONICAL)]);
    let readme =
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"))
            .expect("readme");
    let bodies: Vec<&str> = readme
        .lines()
        .map(str::trim)
        .filter(|l| l.len() > 60 && !l.starts_with('|') && !l.starts_with('#'))
        .take(25)
        .collect();
    assert_eq!(bodies.len(), 25, "the README has enough prose");
    let mut ids = Vec::new();
    for body in &bodies {
        let (status, saved) = a.post(
            "/api/v1/memories",
            &json!({"body": body, "kind": "note", "repo": "checkout-a"}),
        );
        assert_eq!(status, 200, "save: {saved}");
        ids.push(saved["data"]["id"].as_str().expect("id").to_string());
    }
    let (_, found) = a.post(
        "/api/v1/find",
        &json!({"query": "comemory", "repo": "checkout-a"}),
    );
    let query_id = found["data"]["query_id"]
        .as_str()
        .expect("query id")
        .to_string();
    for _ in 0..10 {
        let (status, body) = a.post(
            "/api/v1/feedback",
            &json!({"query_id": query_id, "used": ids}),
        );
        assert_eq!(status, 200, "feedback: {body}");
    }
    let operations: Vec<Value> = changes(&a, &[KIND]).iter().map(operation_of).collect();
    assert_eq!(operations.len(), 250);

    let b = Engine::spawn(&[]);
    let data_dir = b.data_dir();
    let (base, token) = (b.base.clone(), b.token.clone());
    let envelope = json!({"protocol": "replica-v1", "operations": operations});
    let sender = std::thread::spawn(move || {
        reqwest::blocking::Client::new()
            .post(format!("{base}/api/v1/sync/replica/import"))
            .bearer_auth(token)
            .json(&envelope)
            .send()
            .map(|r| r.status().as_u16())
    });
    std::thread::sleep(Duration::from_millis(40));
    let home = b.stop();
    let _ = sender.join().expect("sender thread");

    let rows = feedback_rows(&data_dir);
    for (memory, used, _) in memory_counters(&data_dir) {
        let events = rows
            .iter()
            .filter(|r| r["memory_id"] == json!(memory))
            .count();
        assert_eq!(
            used,
            i64::try_from(events).unwrap(),
            "{memory}: counter and events agree"
        );
    }
    let positions = journal(&data_dir, KIND).len();
    let receipts: i64 = rusqlite::Connection::open(data_dir.join("comemory.db"))
        .expect("open")
        .query_row(
            "SELECT count(*) FROM replica_receipt WHERE disposition = 'accepted'",
            [],
            |r| r.get(0),
        )
        .expect("receipts");
    assert_eq!(rows.len(), positions, "every imported row has its position");
    assert_eq!(
        i64::try_from(rows.len()).unwrap(),
        receipts,
        "and its receipt"
    );

    let b = Engine::spawn_at(&data_dir, &[]);
    import(&b, &operations);
    assert_eq!(
        feedback_rows(&data_dir).len(),
        250,
        "the retry completes it"
    );
    assert!(
        memory_counters(&data_dir)
            .iter()
            .all(|(_, used, _)| *used == 10),
        "each memory counted exactly ten times"
    );
    drop(home);
    drop(workspace);
}
