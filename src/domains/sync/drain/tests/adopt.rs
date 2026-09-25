#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Event adoption over a real database: a run recorded here is captured and
//! queued exactly once, an imported event never is, and a cursor a rebuild
//! loses costs a rescan, never a second outbox row.

use crate::domains::sync::drain::adopt;
use crate::domains::sync::replica::test_support::{BODY, Home, approve};
use crate::store::replica_journal::{self, NewOperation, PayloadRef, ReplicaOp, ReplicaOrigin};

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

    let adopted = adopt::events(&mut home.ctx(), "2026-09-24T10:00:00Z").expect("adopt");
    let again = adopt::events(&mut home.ctx(), "2026-09-24T10:00:01Z").expect("again");

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
    adopt::events(&mut home.ctx(), "2026-09-24T10:00:00Z").expect("adopt");
    // What a rebuild leaves: the outbox kept, the cursor gone.
    home.conn
        .execute(
            "DELETE FROM schema_meta WHERE key = ?1",
            [adopt::EVENTS_THROUGH],
        )
        .expect("forget cursor");

    let rescanned = adopt::events(&mut home.ctx(), "2026-09-24T10:00:01Z").expect("rescan");
    home.save(&format!("{BODY} A second decision."), &["sync"]);
    let later = adopt::events(&mut home.ctx(), "2026-09-24T10:00:02Z").expect("later");

    assert_eq!(rescanned, 0, "the rescan finds every run already queued");
    assert_eq!(later, 1, "a run recorded afterwards is queued");
    assert_eq!(queued_events(&home).len(), 2);
}
