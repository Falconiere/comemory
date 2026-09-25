#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::store::sync_policy_snapshot`] against a real migrated
//! database: one snapshot per key, replaced whole, absent until written.

use std::collections::BTreeMap;

use comemory::store::connection;
use comemory::store::sync_exchange::ExchangeKey;
use comemory::store::sync_policy_snapshot::{self, PolicySnapshot};

fn snapshot(revision: i64, allowed: &[&str]) -> PolicySnapshot {
    PolicySnapshot {
        revision,
        fingerprint: format!("fp-{revision}"),
        allowlist: allowed.iter().map(|s| (*s).to_string()).collect(),
        mappings: BTreeMap::from([("comemory".to_string(), "falconiere/comemory".to_string())]),
        loaded_at: "2026-09-24T10:00:00Z".to_string(),
    }
}

#[test]
fn a_key_without_a_snapshot_has_none_and_a_saved_one_round_trips() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    let key = ExchangeKey::new("http://127.0.0.1:9/api/", "ws_a");
    assert_eq!(
        key.api_url, "http://127.0.0.1:9/api",
        "the base is normalized"
    );
    assert!(
        sync_policy_snapshot::load(&conn, &key)
            .expect("load")
            .is_none()
    );

    let first = snapshot(3, &["falconiere/comemory"]);
    sync_policy_snapshot::save(&conn, &key, &first).expect("save");
    assert_eq!(
        sync_policy_snapshot::load(&conn, &key).expect("load"),
        Some(first)
    );
}

#[test]
fn a_revocation_replaces_the_whole_snapshot_and_other_keys_are_untouched() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    let key = ExchangeKey::new("http://127.0.0.1:9/api", "ws_a");
    let other = ExchangeKey::new("http://127.0.0.1:9/api", "ws_b");
    sync_policy_snapshot::save(&conn, &key, &snapshot(3, &["falconiere/comemory"])).expect("a");
    sync_policy_snapshot::save(&conn, &other, &snapshot(1, &["falconiere/comemory"])).expect("b");

    let revoked = snapshot(4, &[]);
    sync_policy_snapshot::save(&conn, &key, &revoked).expect("revoke");

    assert_eq!(
        sync_policy_snapshot::load(&conn, &key).expect("a"),
        Some(revoked)
    );
    assert_eq!(
        sync_policy_snapshot::load(&conn, &other)
            .expect("b")
            .expect("still there")
            .allowlist,
        vec!["falconiere/comemory".to_string()],
        "a revocation for one workspace never touches another"
    );
}
