#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The capture sweep and the verdict backfill over real recorded runs and
//! real verdicts: what is journalled, what stays local, and that neither walk
//! journals anything twice or moves a counter.

use crate::domains::learning::feedback_tracking::record_with_provenance;
use crate::domains::learning::telemetry::StatsDb;
use crate::domains::sync::replica::{event_capture, manifest};
use crate::store::activity::{self, NewActivityRow};
use crate::store::repository_approval;
use crate::utilities::telemetry::PROV_MANUAL;
use serde_json::Value;

use crate::domains::sync::replica::test_support as support;

use support::{BODY, Home};

fn approve(home: &Home) {
    repository_approval::replace_all(
        &home.conn,
        &[(
            "Falconiere/comemory".to_string(),
            "Falconiere/comemory".to_string(),
        )],
        "2026-09-24T10:00:00Z",
    )
    .expect("approve");
}

fn journalled(home: &Home, kind: &str) -> Vec<Value> {
    let mut statement = home
        .conn
        .prepare(
            "SELECT p.bytes FROM replica_feed f JOIN replica_payload p \
               ON p.digest = f.payload_digest WHERE f.entity_kind = ?1 ORDER BY f.sequence",
        )
        .expect("prepare");
    statement
        .query_map([kind], |r| r.get::<_, String>(0))
        .expect("query")
        .map(|b| serde_json::from_str(&b.expect("row")).expect("json"))
        .collect()
}

fn used_count(home: &Home) -> i64 {
    home.conn
        .query_row("SELECT used_count FROM feedback", [], |r| r.get(0))
        .expect("counter")
}

#[test]
fn a_scoped_approved_run_is_captured_once_with_a_stable_id() {
    let mut home = Home::new();
    approve(&home);
    let id = home.save(BODY, &["sync"]);

    let mut ctx = home.ctx();
    event_capture::advance(&mut ctx).expect("capture");
    let events = journalled(&home, "activity_event");
    assert_eq!(events.len(), 1, "the save run");
    let summary = events[0]["summary"].as_object().expect("summary");
    assert_eq!(summary["id"], Value::String(id));
    for key in summary.keys() {
        assert!(
            ["id", "kind", "tags", "supersedes"].contains(&key.as_str()),
            "save shared {key}"
        );
    }
    let stamped: Option<String> = home
        .conn
        .query_row("SELECT event_id FROM activity_log", [], |r| r.get(0))
        .expect("stamped");
    assert_eq!(stamped.as_deref(), events[0]["event_id"].as_str());

    let mut ctx = home.ctx();
    event_capture::advance(&mut ctx).expect("again");
    assert_eq!(journalled(&home, "activity_event").len(), 1, "never twice");
}

#[test]
fn unapproved_and_bookkeeping_runs_stay_local() {
    let mut home = Home::new();
    home.save(BODY, &["sync"]);
    let mut ctx = home.ctx();
    event_capture::advance(&mut ctx).expect("unapproved");
    assert!(journalled(&home, "activity_event").is_empty());

    approve(&home);
    activity::insert(
        &home.conn,
        &NewActivityRow {
            at: "2026-09-24T10:00:00Z",
            command: "sync.import",
            source: "http",
            actor: None,
            repo: Some("Falconiere/comemory"),
            duration_ms: 4,
            ok: true,
            error_code: None,
            summary: Some(r#"{"entries":1,"applied":1}"#),
            device: None,
            event_id: None,
        },
    )
    .expect("bookkeeping row");
    let mut ctx = home.ctx();
    event_capture::advance(&mut ctx).expect("bookkeeping");
    assert!(
        journalled(&home, "activity_event").is_empty(),
        "the save was walked before approval, and an import is never shared"
    );
}

#[test]
fn the_backfill_shares_unshared_verdicts_once_without_moving_a_counter() {
    let mut home = Home::new();
    let id = home.save(BODY, &["sync"]);
    let mut db = StatsDb::open(home.paths.stats_db()).expect("stats db");
    // Recorded while the repository was unapproved: stored, counted, unshared.
    record_with_provenance(
        &mut db,
        "q-20260924-1a2b3c4d",
        &[id.clone(), id],
        &[],
        PROV_MANUAL,
    )
    .expect("verdicts");
    assert!(journalled(&home, "feedback_event").is_empty());
    approve(&home);

    let mut ctx = home.ctx();
    let progress = event_capture::advance(&mut ctx).expect("backfill");
    assert!(progress.complete());
    assert_eq!(journalled(&home, "feedback_event").len(), 2);
    assert_eq!(used_count(&home), 2, "backfill never touches a counter");

    let mut ctx = home.ctx();
    event_capture::advance(&mut ctx).expect("again");
    assert_eq!(
        journalled(&home, "feedback_event").len(),
        2,
        "one pass, for good"
    );
}

#[test]
fn the_capability_waits_for_the_backfill() {
    let mut home = Home::new();
    let id = home.save(BODY, &["sync"]);
    let mut db = StatsDb::open(home.paths.stats_db()).expect("stats db");
    let many = vec![id; 201];
    record_with_provenance(&mut db, "q-20260924-1a2b3c4d", &many, &[], PROV_MANUAL)
        .expect("verdicts");

    let mut ctx = home.ctx();
    let partway = manifest::run(&mut ctx).expect("manifest");
    assert_eq!(partway.bootstrap.state, "seeding");
    assert!(
        partway.capabilities.is_empty(),
        "a half-walked backfill hides the protocol"
    );

    let mut ctx = home.ctx();
    let finished = manifest::run(&mut ctx).expect("manifest");
    assert_eq!(finished.bootstrap.state, "complete");
    assert_eq!(finished.capabilities, manifest::advertised());
}

#[test]
fn a_lost_capture_cursor_never_shares_a_run_twice() {
    let mut home = Home::new();
    approve(&home);
    home.save(BODY, &["sync"]);
    let mut ctx = home.ctx();
    event_capture::advance(&mut ctx).expect("capture");
    let stamped = |home: &Home| -> String {
        home.conn
            .query_row("SELECT event_id FROM activity_log", [], |r| r.get(0))
            .expect("stamped")
    };
    let first = stamped(&home);
    // What a rebuild leaves: `activity_log` and the feed copied, the capture
    // cursor (in `schema_meta`) gone.
    home.conn
        .execute(
            "DELETE FROM schema_meta WHERE key = ?1",
            [event_capture::ACTIVITY_THROUGH],
        )
        .expect("forget cursor");

    let mut ctx = home.ctx();
    event_capture::advance(&mut ctx).expect("rewalk");

    assert_eq!(
        journalled(&home, "activity_event").len(),
        1,
        "the shared run is not journalled again under a new id"
    );
    assert_eq!(stamped(&home), first, "its event id is kept");
}
