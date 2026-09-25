#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Workspace keying over real saves: a logout stamps what it leaves, a hand
//! replaced credential is caught by the last-used rule, and an entity bound
//! only to another key belongs to it.

use crate::domains::sync::drain::keying;
use crate::domains::sync::replica::test_support::{BODY, Home};
use crate::store::replica_binding::{self, Binding};
use crate::store::replica_outbox;
use crate::store::sync_exchange::{self, ExchangeKey, ExchangeRow};

fn stamps(home: &Home) -> Vec<Option<String>> {
    replica_outbox::pending(&home.conn, 50)
        .expect("pending")
        .into_iter()
        .map(|r| r.workspace_id)
        .collect()
}

#[test]
fn a_logout_stamps_every_unsent_row_with_the_key_it_leaves() {
    let mut home = Home::new();
    home.save(BODY, &["sync"]);
    home.save("A second pending memory.", &["sync"]);
    let leaving = ExchangeKey::new("http://127.0.0.1:9/api", "ws_a");

    assert_eq!(
        keying::stamp_outgoing(&home.conn, &leaving, "t").expect("stamp"),
        2
    );
    assert_eq!(
        stamps(&home),
        vec![Some("ws_a".to_string()), Some("ws_a".to_string())]
    );
}

#[test]
fn a_hand_replaced_credential_stamps_what_the_old_key_last_saw() {
    let mut home = Home::new();
    home.save(BODY, &["sync"]);
    let old = ExchangeKey::new("http://127.0.0.1:9/api", "ws_a");
    let mut row = ExchangeRow::fresh(&old);
    row.last_session_at = Some("2999-01-01T00:00:00Z".to_string());
    sync_exchange::save(&home.conn, &row, "t").expect("old key used last");

    let new = ExchangeKey::new("http://127.0.0.1:9/api", "ws_b");
    assert_eq!(
        keying::apply_last_used_rule(&home.conn, &new, "t").expect("rule"),
        1
    );
    assert_eq!(stamps(&home), vec![Some("ws_a".to_string())]);
    assert_eq!(
        keying::apply_last_used_rule(&home.conn, &old, "t").expect("same key"),
        0,
        "the key used last owes nothing to itself"
    );
}

#[test]
fn an_entity_bound_only_elsewhere_belongs_there() {
    let home = Home::new();
    let a = ExchangeKey::new("http://127.0.0.1:9/api", "ws_a");
    let b = ExchangeKey::new("http://127.0.0.1:9/api", "ws_b");
    let binding = Binding {
        entity_kind: "memory".into(),
        entity_key: "a1b2c3d4".into(),
        synced_digest: Some("d".repeat(64)),
        synced_deleted: false,
        synced_sequence: Some(3),
        synced_epoch: None,
    };
    replica_binding::upsert(&home.conn, &a, &binding, "t").expect("bind to a");

    assert_eq!(
        keying::foreign_owner(&home.conn, &b, "memory", "a1b2c3d4").expect("owner"),
        Some(a.clone())
    );
    assert_eq!(
        keying::foreign_owner(&home.conn, &a, "memory", "a1b2c3d4").expect("own"),
        None
    );
    assert_eq!(
        keying::foreign_owner(&home.conn, &b, "memory", "ffffffff").expect("none"),
        None
    );
}
