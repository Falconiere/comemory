#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::store::replica_pull_hold`] against a real migrated
//! database: holds are per key and epoch, counted by position, and a
//! rebootstrap clears the replica ones while legacy holds stay counted.

use comemory::store::connection;
use comemory::store::replica_pull_hold::{self, LEGACY_EPOCH, PullHold, Which};
use comemory::store::sync_exchange::ExchangeKey;

fn hold(from: i64, to: i64, reason: &str) -> PullHold {
    PullHold {
        from_sequence: from,
        to_sequence: to,
        reason: reason.to_string(),
        entity_kind: (from == to).then(|| "memory".to_string()),
        entity_key: (from == to).then(|| format!("m{from:07}")),
        repository: (reason == "policy").then(|| "acme/private".to_string()),
        policy_revision: (reason == "server_withheld").then_some(3),
    }
}

#[test]
fn holds_list_in_position_order_and_count_every_position() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    let key = ExchangeKey::new("http://127.0.0.1:9/api", "ws_a");
    let epoch = "0123456789abcdef0123456789abcdef";
    for h in [
        hold(40, 40, "policy"),
        hold(12, 12, "pending_local"),
        hold(20, 29, "server_withheld"),
    ] {
        replica_pull_hold::record(&conn, &key, epoch, &h, "t").expect("record");
    }

    let listed = replica_pull_hold::list(&conn, &key, epoch).expect("list");
    assert_eq!(
        listed.iter().map(|h| h.from_sequence).collect::<Vec<_>>(),
        vec![12, 20, 40]
    );
    assert_eq!(listed[2].repository.as_deref(), Some("acme/private"));
    assert_eq!(
        replica_pull_hold::counts(&conn, &key).expect("counts"),
        vec![
            ("pending_local".to_string(), 1),
            ("policy".to_string(), 1),
            ("server_withheld".to_string(), 10),
        ],
        "a withheld range counts every position in it"
    );
}

#[test]
fn a_rebootstrap_clears_replica_holds_and_keeps_legacy_ones() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    let key = ExchangeKey::new("http://127.0.0.1:9/api", "ws_a");
    let other = ExchangeKey::new("http://127.0.0.1:9/api", "ws_b");
    replica_pull_hold::record(&conn, &key, "epoch-old", &hold(5, 5, "policy"), "t").expect("a");
    replica_pull_hold::record(&conn, &key, LEGACY_EPOCH, &hold(7, 7, "secret"), "t").expect("l");
    replica_pull_hold::record(&conn, &other, "epoch-old", &hold(5, 5, "policy"), "t").expect("b");

    assert_eq!(
        replica_pull_hold::drop_holds(&conn, &key, Which::Replica).expect("clear"),
        1
    );
    assert!(
        replica_pull_hold::list(&conn, &key, "epoch-old")
            .expect("list")
            .is_empty()
    );
    assert_eq!(
        replica_pull_hold::list(&conn, &key, LEGACY_EPOCH)
            .expect("legacy")
            .len(),
        1,
        "a legacy hold is counted, never rewound, and survives a replica rebootstrap"
    );
    assert_eq!(
        replica_pull_hold::list(&conn, &other, "epoch-old")
            .expect("other")
            .len(),
        1,
        "another key's holds are untouched"
    );
}

#[test]
fn recording_the_same_position_replaces_it_and_a_bad_reason_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    let key = ExchangeKey::new("http://127.0.0.1:9/api", "ws_a");
    replica_pull_hold::record(&conn, &key, "e", &hold(9, 9, "pending_local"), "t").expect("one");
    replica_pull_hold::record(&conn, &key, "e", &hold(9, 9, "policy"), "t").expect("again");
    let listed = replica_pull_hold::list(&conn, &key, "e").expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].reason, "policy");
    assert!(replica_pull_hold::record(&conn, &key, "e", &hold(10, 10, "bored"), "t").is_err());
}
