#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The exchange client (#255), part 8: feedback and activity events (#254)
//! drain like every other kind (AC-19). A verdict and the runs recorded on
//! one client reach a peer through the hub and are counted there once; the
//! echo of a client's own events, and every later pass on either side, count
//! nothing again.
//!
//! Real `comemory serve` hubs, the real CLI binary, real SQLite, real HTTP.

#[path = "common/exchange_support.rs"]
mod exchange_support;
#[path = "common/fault_proxy.rs"]
mod fault_proxy;
#[path = "common/replica_support.rs"]
mod replica_support;

use exchange_support::{Client, Hub, REPO, WORKSPACE};
use serde_json::json;

/// A client logged into `hub` with `REPO` approved — the same shape every
/// other exchange suite uses.
fn approved_client(hub: &Hub) -> Client {
    let client = Client::new();
    client.login(hub);
    client.approve(&hub.api_url(), WORKSPACE, &[REPO], 1);
    client
}

/// A decision the verdict is about, and a query that finds it lexically.
const BODY: &str = "Replicated verdicts are counted once on every engine that receives them.";
const QUERY: &str = "replicated verdicts counted once";

/// One integer `sql` selects off `conn`.
fn scalar(conn: &rusqlite::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0))
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

/// The event ids this engine shared from its own activity, sorted.
fn shared_run_ids(client: &Client) -> Vec<String> {
    let conn = client.open();
    let mut statement = conn
        .prepare(
            "SELECT event_id FROM activity_log \
             WHERE event_id IS NOT NULL AND device IS NULL ORDER BY event_id",
        )
        .expect("prepare");
    statement
        .query_map([], |r| r.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect")
}

/// What either side of the exchange counts: `(verdict events, used_count of
/// the memory, activity rows, event rows still owed)`.
fn counts(client: &Client, memory: &str) -> (i64, i64, i64, i64) {
    let conn = client.open();
    (
        scalar(&conn, "SELECT count(*) FROM feedback_events"),
        conn.query_row(
            "SELECT used_count FROM feedback WHERE memory_id = ?1",
            [memory],
            |r| r.get(0),
        )
        .unwrap_or(0),
        scalar(
            &conn,
            "SELECT count(*) FROM activity_log WHERE event_id IS NOT NULL",
        ),
        scalar(
            &conn,
            "SELECT count(*) FROM replica_operation WHERE state != 'accepted' \
             AND entity_kind IN ('feedback_event', 'activity_event')",
        ),
    )
}

#[test]
fn a_verdict_and_its_runs_reach_the_peer_and_count_once() {
    let hub = Hub::start();
    let a = approved_client(&hub);
    let b = approved_client(&hub);
    let memory = a.save(BODY, REPO);
    let found = a.cli(&["find", QUERY, "--repo", REPO]);
    let query_id = found["query_id"].as_str().expect("query id").to_string();
    assert!(
        found["hits"]
            .as_array()
            .expect("hits")
            .iter()
            .any(|h| h["id"] == json!(memory)),
        "the find returned the saved memory: {found}"
    );
    a.cli(&["feedback", &query_id, "--used", &memory]);

    a.sync();
    b.sync();

    let verdict: String = a
        .open()
        .query_row("SELECT event_id FROM feedback_events", [], |r| r.get(0))
        .expect("A's verdict was shared");
    let received: String = b
        .open()
        .query_row("SELECT event_id FROM feedback_events", [], |r| r.get(0))
        .expect("B received the verdict");
    assert_eq!(received, verdict, "the same event, under its own id");
    let runs = shared_run_ids(&a);
    assert!(!runs.is_empty(), "A's scoped runs were captured and shared");
    let received_runs: Vec<String> = {
        let conn = b.open();
        let mut statement = conn
            .prepare("SELECT event_id FROM activity_log WHERE device IS NOT NULL ORDER BY event_id")
            .expect("prepare");
        statement
            .query_map([], |r| r.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("collect")
    };
    assert_eq!(received_runs, runs, "every shared run arrived once");
    let settled_a = counts(&a, &memory);
    let settled_b = counts(&b, &memory);
    assert_eq!(settled_a.0, 1);
    assert_eq!(settled_a.1, 1, "A counted its own verdict once");
    assert_eq!(settled_a.3, 0, "every event A owed was accepted");
    assert_eq!((settled_b.0, settled_b.1), (1, 1), "B counted it once too");
    assert_eq!(settled_b.3, 0, "B re-offers nothing it received");

    // The echo of A's own events, and more passes both ways: a fixed point.
    for _ in 0..2 {
        a.sync();
        b.sync();
    }
    assert_eq!(counts(&a, &memory), settled_a, "A counts nothing again");
    assert_eq!(counts(&b, &memory), settled_b, "B counts nothing again");
    let hub_db = hub.db();
    assert_eq!(
        scalar(
            &hub_db,
            "SELECT count(*) FROM replica_feed WHERE entity_kind = 'feedback_event'"
        ),
        1,
        "the hub holds the verdict once"
    );
    assert_eq!(
        scalar(
            &hub_db,
            "SELECT count(*) FROM replica_feed WHERE entity_kind = 'activity_event'"
        ),
        i64::try_from(runs.len()).expect("run count"),
        "and each run once"
    );
}
