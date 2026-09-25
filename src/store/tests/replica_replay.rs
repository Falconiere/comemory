#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::store::replica_replay`] against a real migrated
//! database: only an entity's last position survives the scan, and the apply
//! phase reads them back in position order.

use comemory::store::connection;
use comemory::store::replica_replay::{self, ReplayRow};
use comemory::store::sync_exchange::ExchangeKey;

fn row(entity: &str, sequence: i64) -> ReplayRow {
    ReplayRow {
        entity_kind: "memory".to_string(),
        entity_key: entity.to_string(),
        sequence,
        entry_json: format!("{{\"sequence\":{sequence}}}"),
    }
}

#[test]
fn a_later_position_replaces_an_earlier_one_and_never_the_reverse() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    let key = ExchangeKey::new("http://127.0.0.1:9/api", "ws_a");
    assert!(replica_replay::offer(&conn, &key, &row("m1", 10)).expect("10"));
    assert!(replica_replay::offer(&conn, &key, &row("m1", 11)).expect("11"));
    assert!(
        !replica_replay::offer(&conn, &key, &row("m1", 4)).expect("4"),
        "a page read again after a resume cannot put an older entry back"
    );
    assert!(replica_replay::offer(&conn, &key, &row("m2", 7)).expect("m2"));

    let next = replica_replay::next(&conn, &key, 10).expect("next");
    assert_eq!(
        next,
        vec![row("m2", 7), row("m1", 11)],
        "position order, one per entity"
    );
}

#[test]
fn applied_rows_are_removed_and_clear_starts_over() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    let key = ExchangeKey::new("http://127.0.0.1:9/api", "ws_a");
    let other = ExchangeKey::new("http://127.0.0.1:9/api", "ws_b");
    for (k, entity, seq) in [(&key, "m1", 1), (&key, "m2", 2), (&other, "m1", 3)] {
        replica_replay::offer(&conn, k, &row(entity, seq)).expect("offer");
    }
    replica_replay::clear(&conn, &key, Some(("memory", "m1"))).expect("remove");
    assert_eq!(
        replica_replay::next(&conn, &key, 10).expect("next"),
        vec![row("m2", 2)]
    );
    assert_eq!(replica_replay::clear(&conn, &key, None).expect("clear"), 1);
    assert!(
        replica_replay::next(&conn, &key, 10)
            .expect("empty")
            .is_empty()
    );
    assert_eq!(
        replica_replay::next(&conn, &other, 10)
            .expect("other")
            .len(),
        1,
        "another key's replay is untouched"
    );
}
