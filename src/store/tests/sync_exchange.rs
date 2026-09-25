#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::store::sync_exchange`] against a real migrated
//! database: one row per session key, written whole and read back whole, and
//! the hub predicate that scopes the pending-upload refusal.

use comemory::store::connection;
use comemory::store::sync_exchange::{self, ExchangeKey, ExchangeRow};
use rusqlite::Connection;

fn db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

#[test]
fn a_row_round_trips_every_field() {
    let (_dir, conn) = db();
    let mut row = ExchangeRow::fresh(&ExchangeKey::new("https://api.comemory.io", "ws_a"));
    row.protocol = Some("replica-v1".into());
    row.selected_at = Some("2026-09-24T10:00:00Z".into());
    row.upgrade_through = Some(412);
    row.replay_kind = Some("repair:document_revision".into());
    row.replay_state = Some("scanning".into());
    row.replay_scan_through = Some(90);
    row.replay_target = Some(412);
    row.network_state = "backoff".into();
    row.retry_at = Some("2026-09-24T10:00:04Z".into());
    row.consecutive_failures = 2;
    row.last_error = Some("HTTP 502".into());
    row.upstream_head = Some(412);
    row.stall_sequence = Some(300);
    row.stall_reason = Some("incompatible_version".into());
    sync_exchange::save(&conn, &row, "2026-09-24T10:00:01Z").expect("save");

    let loaded = sync_exchange::load(&conn, &ExchangeKey::new("https://api.comemory.io", "ws_a"))
        .expect("load")
        .expect("row");
    assert_eq!(loaded, row);
}

#[test]
fn saving_again_replaces_the_row_instead_of_adding_one() {
    let (_dir, conn) = db();
    let mut row = ExchangeRow::fresh(&ExchangeKey::new("https://api.comemory.io", "ws_a"));
    sync_exchange::save(&conn, &row, "t1").expect("save");
    row.network_state = "auth_suspended".into();
    row.suspended_fingerprint = Some("abc".into());
    sync_exchange::save(&conn, &row, "t2").expect("save again");

    assert_eq!(sync_exchange::all(&conn).expect("all"), vec![row]);
}

#[test]
fn only_a_replica_selection_makes_this_engine_a_client() {
    let (_dir, conn) = db();
    assert!(!sync_exchange::has_replica_upstream(&conn).expect("empty"));

    let mut legacy = ExchangeRow::fresh(&ExchangeKey::new("https://api.comemory.io", "ws_a"));
    legacy.protocol = Some("legacy".into());
    sync_exchange::save(&conn, &legacy, "t").expect("save legacy");
    assert!(
        !sync_exchange::has_replica_upstream(&conn).expect("legacy only"),
        "an engine on the old protocol owes no replica uploads to anyone"
    );

    let mut replica = ExchangeRow::fresh(&ExchangeKey::new("http://127.0.0.1:9/api", "ws_b"));
    replica.protocol = Some("replica-v1".into());
    sync_exchange::save(&conn, &replica, "t").expect("save replica");
    assert!(sync_exchange::has_replica_upstream(&conn).expect("replica"));
}

#[test]
fn a_state_the_schema_refuses_is_an_error_not_a_silent_write() {
    let (_dir, conn) = db();
    let mut row = ExchangeRow::fresh(&ExchangeKey::new("https://api.comemory.io", "ws_a"));
    row.network_state = "sleeping".into();
    assert!(sync_exchange::save(&conn, &row, "t").is_err());
    assert!(
        sync_exchange::load(&conn, &ExchangeKey::new("https://api.comemory.io", "ws_a"))
            .expect("load")
            .is_none(),
        "the refused write left nothing behind"
    );
}
