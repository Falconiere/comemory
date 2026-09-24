#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Feedback verdicts over the real surface (#254), second half: backfill of
//! retained history (AC-6), what a replicated verdict may never become (AC-7),
//! purge (AC-12), rebuild (AC-15) and a query row evicted before the verdict
//! travels (AC-16). Identity, replay and atomicity are `replica_events.rs`.

#[path = "common/replica_support.rs"]
mod replica_support;

#[path = "common/replica_events_support.rs"]
mod replica_events_support;

use std::time::Duration;

use replica_events_support::{
    MEMORY_QUERY, age, changes, code_counters, count, device_of, dispositions, feedback,
    feedback_rows, import, memory_counters, operation_of, pair, transfer,
};
use replica_support::{BODY, Engine, cli_raw, cli_with_env};
use serde_json::{Value, json};

const KIND: &str = "feedback_event";

#[test]
fn backfill_shares_only_retained_memory_verdicts_once_and_never_moves_a_counter() {
    let p = pair();
    feedback(&p.a, None, &p.scene, false);
    feedback(&p.a, None, &p.scene, true);
    feedback(&p.a, None, &p.scene, false);
    // The state a database upgraded from 0025 is in: verdicts with no event id
    // and nothing journalled for them.
    p.a.db()
        .execute_batch(
            "UPDATE feedback_events SET event_id = NULL, device = NULL, surface = NULL, actor = NULL; \
             DELETE FROM replica_feed WHERE entity_kind = 'feedback_event'; \
             DELETE FROM replica_revision WHERE entity_kind = 'feedback_event'; \
             DELETE FROM replica_payload WHERE entity_kind = 'feedback_event'; \
             DELETE FROM schema_meta WHERE key LIKE 'replica_feedback_backfill%';",
        )
        .expect("upgrade state");
    let oldest: i64 =
        p.a.db()
            .query_row("SELECT min(id) FROM feedback_events", [], |r| r.get(0))
            .expect("oldest");
    age(
        &p.a.data_dir(),
        "feedback_events",
        &format!("id = {oldest}"),
    );
    let (code, _) = cli_with_env(&p.a.data_dir(), &[], &["gc"]);
    assert_eq!(code, 0);
    let before = (
        memory_counters(&p.a.data_dir()),
        code_counters(&p.a.data_dir()),
    );
    assert_eq!(
        before.0[0].1, 3,
        "the evicted verdict's contribution stays in the counter"
    );

    let first = changes(&p.a, &[KIND]);
    assert_eq!(
        first.len(),
        2,
        "the two retained memory verdicts, not the evicted one or the code one"
    );
    assert!(
        first
            .iter()
            .all(|e| e["payload"]["target"]["kind"] == json!("memory"))
    );
    let stamped: Vec<Value> = feedback_rows(&p.a.data_dir())
        .into_iter()
        .filter(|r| r["event_id"].is_string())
        .collect();
    assert_eq!(stamped.len(), 2);
    let code_rows: Vec<Value> = feedback_rows(&p.a.data_dir())
        .into_iter()
        .filter(|r| r["target_kind"] == json!("code"))
        .collect();
    assert!(
        code_rows.iter().all(|r| r["event_id"].is_null()),
        "legacy code verdicts stay local"
    );

    let second = changes(&p.a, &[KIND]);
    assert_eq!(second.len(), 2, "a second pass journals nothing");
    let again: Vec<Value> = feedback_rows(&p.a.data_dir())
        .into_iter()
        .filter(|r| r["event_id"].is_string())
        .collect();
    assert_eq!(again, stamped, "and changes no id");
    assert_eq!(
        (
            memory_counters(&p.a.data_dir()),
            code_counters(&p.a.data_dir())
        ),
        before,
        "backfill never touches a counter"
    );

    transfer(&p.a, &p.b, &[KIND]);
    assert_eq!(
        memory_counters(&p.b.data_dir()),
        vec![(p.scene.memory.clone(), 2, 0)],
        "the receiver counts retained events, not the sender's counter"
    );
}

#[test]
fn replicated_verdicts_keep_provenance_and_never_become_local_ground_truth() {
    let p = pair();
    feedback(&p.a, None, &p.scene, false);
    let (status, body) = p.a.post(
        "/api/v1/feedback",
        &json!({"query_id": p.scene.query_id, "used": [p.scene.memory], "source": "implicit"}),
    );
    assert_eq!(status, 200, "implicit verdict: {body}");

    let own =
        p.b.cli(&["save", BODY, "--kind", "decision", "--repo", "checkout-b"]);
    let own_id = own["id"].as_str().expect("id").to_string();
    let found = p.b.cli(&["find", MEMORY_QUERY, "--repo", "checkout-b"]);
    let own_query = found["query_id"].as_str().expect("query").to_string();
    let recall_before = p.b.cli(&["recall-status", "--since", "2000-01-01"]);

    transfer(&p.a, &p.b, &[KIND]);
    let provenances: Vec<Value> = feedback_rows(&p.b.data_dir())
        .into_iter()
        .map(|r| r["provenance"].clone())
        .collect();
    assert_eq!(provenances, vec![json!("manual"), json!("implicit")]);

    let recall_after = p.b.cli(&["recall-status", "--since", "2000-01-01"]);
    assert_eq!(
        recall_after, recall_before,
        "an import is not a local recall session"
    );
    assert!(
        recall_after["pending"]
            .as_array()
            .expect("pending")
            .iter()
            .any(|q| q["query_id"] == json!(own_query)),
        "the receiver's own query is still owed a verdict: {recall_after}"
    );

    let (code, _, stderr) = cli_raw(&p.b.data_dir(), &["eval"]);
    assert_ne!(code, 0, "no replicated verdict is a golden pair");
    assert!(stderr.contains("no golden pairs"), "{stderr}");

    p.b.cli(&["feedback", &own_query, "--used", &own_id]);
    let eval = p.b.cli(&["eval"]);
    assert_eq!(
        eval["queries"],
        json!(1),
        "the receiver's own verdict still is — and it is the only one: {eval}"
    );
}

#[test]
fn purging_a_memory_erases_its_verdicts_from_the_journal_for_good() {
    let p = pair();
    feedback(&p.a, None, &p.scene, false);
    let offered: Vec<Value> = changes(&p.a, &[KIND]).iter().map(operation_of).collect();
    transfer(&p.a, &p.b, &[KIND]);

    p.a.cli(&["delete", &p.scene.memory]);
    let trash = p.a.data_dir().join("memories").join(".trash");
    for entry in std::fs::read_dir(&trash).expect("trash") {
        let path = entry.expect("entry").path();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open trash file")
            .set_modified(std::time::SystemTime::now() - Duration::from_hours(90 * 24))
            .expect("age trash file");
    }
    p.a.db()
        .execute(
            "UPDATE memories SET deleted_at = '2020-01-01T00:00:00Z' WHERE id = ?1",
            [&p.scene.memory],
        )
        .expect("age deletion");
    let (code, _) = cli_with_env(&p.a.data_dir(), &[], &["gc"]);
    assert_eq!(code, 0);

    let entries = changes(&p.a, &[KIND]);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["payload_state"], json!("erased"));
    assert!(entries[0]["payload"].is_null());
    assert_eq!(
        dispositions(&import(&p.a, &offered)),
        vec!["payload_erased"],
        "a replay restores nothing"
    );
    assert_eq!(count(&p.a.data_dir(), "feedback_events"), 0);
    assert!(memory_counters(&p.a.data_dir()).is_empty());
}

#[test]
fn rebuild_keeps_the_device_the_event_ids_and_the_activity_history() {
    let p = pair();
    feedback(&p.a, None, &p.scene, true);
    changes(&p.a, &[]);
    transfer(&p.a, &p.b, &[KIND]);
    let device = device_of(&p.a.data_dir());
    let events = feedback_rows(&p.a.data_dir());
    let activity = count(&p.a.data_dir(), "activity_log");
    assert!(
        activity > 0,
        "the searches and the verdict recorded activity"
    );

    let stopped = Engine::reopen(p.a);
    let (code, _, stderr) = cli_raw(&stopped.data_dir(), &["rebuild"]);
    assert_eq!(code, 0, "rebuild: {stderr}");
    assert_eq!(device_of(&stopped.data_dir()), device);
    assert_eq!(feedback_rows(&stopped.data_dir()), events);
    assert_eq!(count(&stopped.data_dir(), "activity_log"), activity);

    let a = Engine::spawn_at(&stopped.data_dir(), &[]);
    assert_eq!(
        dispositions(&transfer(&p.b, &a, &[KIND])),
        vec!["duplicate", "duplicate"],
        "the rebuilt engine still knows its own events"
    );
}

#[test]
fn a_verdict_outlives_the_query_row_it_cites() {
    let p = pair();
    feedback(&p.a, None, &p.scene, false);
    age(
        &p.a.data_dir(),
        "retrieval_log",
        &format!("query_id = '{}'", p.scene.query_id),
    );
    let (code, _) = cli_with_env(&p.a.data_dir(), &[], &["gc"]);
    assert_eq!(code, 0);
    let gone: i64 =
        p.a.db()
            .query_row(
                "SELECT count(*) FROM retrieval_log WHERE query_id = ?1",
                [&p.scene.query_id],
                |r| r.get(0),
            )
            .expect("count");
    assert_eq!(gone, 0, "the query row was evicted");

    assert_eq!(
        dispositions(&transfer(&p.a, &p.b, &[KIND])),
        vec!["accepted"]
    );
    let device = device_of(&p.a.data_dir());
    let rows = feedback_rows(&p.b.data_dir());
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]["query_id"],
        json!(format!("{device}:{}", p.scene.query_id))
    );
    assert_eq!(rows[0]["provenance"], json!("manual"));
    assert_eq!(
        memory_counters(&p.b.data_dir()),
        vec![(p.scene.memory.clone(), 1, 0)]
    );
    assert_eq!(count(&p.b.data_dir(), "retrieval_log"), 0);
}
