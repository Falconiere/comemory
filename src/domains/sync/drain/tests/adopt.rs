#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Event adoption over a real database: a run recorded here is captured and
//! queued exactly once, an imported event never is, a cursor a rebuild loses
//! costs a rescan, never a second outbox row, and the inline push adopts one
//! page per kind, leaving the rest to the next pass.

use crate::domains::sync::drain::adopt::{self, Reach};
use crate::domains::sync::replica::test_support::{BODY, Home, approve};
use crate::store::replica_journal::{
    self, LocalEvent, NewOperation, PayloadRef, ReplicaOp, ReplicaOrigin,
};
use crate::utilities::canonical_json::bytes_and_digest;

/// One adoption pass over `home`, the way a push pass runs it.
fn events(home: &mut Home, at: &str, reach: Reach) -> usize {
    adopt::events((&home.paths, &home.cfg), &mut home.conn, at, reach).expect("adopt")
}

/// `(entity_kind, state, repository)` of every outbox row of an event kind.
fn queued_events(home: &Home) -> Vec<(String, String, Option<String>)> {
    let mut statement = home
        .conn
        .prepare(
            "SELECT entity_kind, state, repository FROM replica_operation \
             WHERE entity_kind IN ('feedback_event', 'activity_event') ORDER BY rowid",
        )
        .expect("prepare");
    statement
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect")
}

/// Journal an event a peer shared, the way the import path lands one.
fn import_peer_event(home: &Home) {
    let digest = "a".repeat(64);
    replica_journal::append(
        &home.conn,
        &home.epoch(),
        &NewOperation {
            operation_id: "op-20260924-peeractivity000000000000000001",
            entity_kind: "activity_event",
            entity_key: "ev-20260924-peer0001",
            op: ReplicaOp::Upsert,
            payload: Some(PayloadRef {
                digest: &digest,
                bytes: "{}",
            }),
            schema_version: 1,
            repository: Some("Falconiere/comemory"),
            origin: ReplicaOrigin::Sync,
            at: "2026-09-24T09:00:00Z",
        },
    )
    .expect("imported event");
}

#[test]
fn a_local_run_is_captured_and_queued_once_and_an_imported_one_never() {
    let mut home = Home::new();
    approve(&home.conn, "Falconiere/comemory");
    import_peer_event(&home);
    home.save(BODY, &["sync"]);

    let adopted = events(&mut home, "2026-09-24T10:00:00Z", Reach::All);
    let again = events(&mut home, "2026-09-24T10:00:01Z", Reach::All);

    assert_eq!(adopted, 1, "the save run, captured by the same call");
    assert_eq!(again, 0, "a second pass queues nothing twice");
    assert_eq!(
        queued_events(&home),
        [(
            "activity_event".to_string(),
            "pending".to_string(),
            Some("Falconiere/comemory".to_string())
        )],
        "only the local run, under its canonical repository"
    );
}

#[test]
fn a_lost_cursor_rescans_without_queueing_twice_and_new_runs_still_flow() {
    let mut home = Home::new();
    approve(&home.conn, "Falconiere/comemory");
    home.save(BODY, &["sync"]);
    events(&mut home, "2026-09-24T10:00:00Z", Reach::All);
    // What a rebuild leaves: the outbox kept, the cursor gone.
    home.conn
        .execute(
            "DELETE FROM schema_meta WHERE key = ?1",
            [format!("{}:activity_event", adopt::EVENTS_THROUGH)],
        )
        .expect("forget cursor");

    let rescanned = events(&mut home, "2026-09-24T10:00:01Z", Reach::All);
    home.save(&format!("{BODY} A second decision."), &["sync"]);
    let later = events(&mut home, "2026-09-24T10:00:02Z", Reach::All);

    assert_eq!(rescanned, 0, "the rescan finds every run already queued");
    assert_eq!(later, 1, "a run recorded afterwards is queued");
    assert_eq!(queued_events(&home).len(), 2);
}

/// Journal `count` local activity events through the real journal API, as
/// #254's capture does, without a run behind each.
fn journal_events(home: &Home, count: usize) {
    let tx = home.conn.unchecked_transaction().expect("tx");
    for n in 0..count {
        let event_id = format!("ev-20260924-{n:08x}");
        let (bytes, digest) =
            bytes_and_digest(&serde_json::json!({"event_id": event_id, "n": n})).expect("payload");
        let bytes = String::from_utf8(bytes).expect("utf8");
        replica_journal::append_local_event(
            &tx,
            &LocalEvent {
                entity_kind: "activity_event",
                event_id: &event_id,
                schema_version: 1,
                repository: "Falconiere/comemory",
                payload: PayloadRef {
                    digest: &digest,
                    bytes: &bytes,
                },
                at: "2026-09-24T10:00:00Z",
            },
        )
        .expect("journal event");
    }
    tx.commit().expect("commit");
}

#[test]
fn an_inline_pass_adopts_one_page_per_kind_and_the_next_pass_the_rest() {
    let mut home = Home::new();
    journal_events(&home, 501);

    let inline = events(&mut home, "2026-09-24T10:00:00Z", Reach::OnePage);
    let next = events(&mut home, "2026-09-24T10:00:01Z", Reach::All);
    let after = events(&mut home, "2026-09-24T10:00:02Z", Reach::All);

    assert_eq!(inline, 500, "one page, however large the backlog");
    assert_eq!(next, 1, "the cursor carried the rest to the next pass");
    assert_eq!(after, 0);
    assert_eq!(queued_events(&home).len(), 501);
}
