#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::store::replica_binding`] against a real migrated
//! database: per-key bindings, the keys an entity is bound to, and the
//! rebootstrap that forgets positions but keeps digests.

use comemory::store::connection;
use comemory::store::replica_binding::{self, Binding};
use comemory::store::sync_exchange::ExchangeKey;

fn binding(key: &str, digest: Option<&str>, sequence: i64) -> Binding {
    Binding {
        entity_kind: "memory".to_string(),
        entity_key: key.to_string(),
        synced_digest: digest.map(str::to_string),
        synced_deleted: digest.is_none(),
        synced_sequence: Some(sequence),
        synced_epoch: Some("0123456789abcdef0123456789abcdef".to_string()),
    }
}

#[test]
fn a_binding_is_replaced_per_entity_and_read_back() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    let key = ExchangeKey::new("http://127.0.0.1:9/api", "ws_a");
    replica_binding::upsert(&conn, &key, &binding("a1b2c3d4", Some("d1"), 10), "t1").expect("1");
    replica_binding::upsert(&conn, &key, &binding("a1b2c3d4", None, 14), "t2").expect("2");

    let got = replica_binding::get(&conn, &key, "memory", "a1b2c3d4")
        .expect("get")
        .expect("bound");
    assert_eq!(
        got,
        binding("a1b2c3d4", None, 14),
        "the tombstone replaced the upsert"
    );
    assert_eq!(replica_binding::all(&conn, &key).expect("all").len(), 1);
}

#[test]
fn an_entity_knows_every_key_it_is_bound_to_newest_first() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    let a = ExchangeKey::new("http://127.0.0.1:9/api", "ws_a");
    let b = ExchangeKey::new("http://127.0.0.1:9/api", "ws_b");
    replica_binding::upsert(
        &conn,
        &a,
        &binding("a1b2c3d4", Some("d1"), 10),
        "2026-09-24T10:00:00Z",
    )
    .expect("a");
    replica_binding::upsert(
        &conn,
        &b,
        &binding("a1b2c3d4", Some("d1"), 3),
        "2026-09-24T11:00:00Z",
    )
    .expect("b");

    assert_eq!(
        replica_binding::keys_for(&conn, "memory", "a1b2c3d4").expect("keys"),
        vec![b, a]
    );
    assert!(
        replica_binding::keys_for(&conn, "memory", "ffffffff")
            .expect("none")
            .is_empty()
    );
}

#[test]
fn a_rebootstrap_forgets_positions_and_keeps_digests() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    let key = ExchangeKey::new("http://127.0.0.1:9/api", "ws_a");
    replica_binding::upsert(&conn, &key, &binding("a1b2c3d4", Some("d1"), 10), "t").expect("1");
    replica_binding::upsert(&conn, &key, &binding("b2c3d4e5", Some("d2"), 11), "t").expect("2");

    assert_eq!(
        replica_binding::clear_sequences(&conn, &key).expect("clear"),
        2
    );
    let all = replica_binding::all(&conn, &key).expect("all");
    assert!(
        all.iter()
            .all(|b| b.synced_sequence.is_none() && b.synced_epoch.is_none())
    );
    assert_eq!(
        all.iter()
            .map(|b| b.synced_digest.clone())
            .collect::<Vec<_>>(),
        vec![Some("d1".to_string()), Some("d2".to_string())]
    );
}
