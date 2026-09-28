#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Adopting memory seed positions the outbox never saw because this engine
//! seeded them before it was ever a `replica-v1` client: sent once the key
//! is a client, never twice, never over a bound entity, and never ahead of a
//! later local operation on the same entity.

use crate::domains::sync::drain::adopt_seeds;
use crate::domains::sync::replica::bootstrap;
use crate::domains::sync::replica::test_support::{Home, forget_the_journal};
use crate::store::replica_binding::{self, Binding};
use crate::store::replica_outbox::{self, Scope};
use crate::store::sync_exchange::ExchangeKey;

const AT: &str = "2026-09-25T10:00:00Z";

fn key() -> ExchangeKey {
    ExchangeKey::new("http://127.0.0.1:9/api", "ws_a")
}

/// Seed one memory the way an engine that was not yet a client does: the
/// outbox row is discarded on the spot, exactly what makes it need adoption
/// later.
fn seed_one_pre_client(home: &mut Home) -> String {
    let id = home.save("Seeded before this engine ever logged in.", &["sync"]);
    forget_the_journal(home);
    let mut ctx = home.ctx();
    bootstrap::advance(&mut ctx).expect("seed");
    assert!(
        replica_outbox::read(&home.conn, Scope::Entity("memory", &id), 1)
            .expect("read")
            .is_empty(),
        "the pre-client seed must have discarded its own outbox row"
    );
    id
}

#[test]
fn a_pre_client_seed_is_adopted_once_the_key_is_a_client() {
    let mut home = Home::new();
    let id = seed_one_pre_client(&mut home);
    home.make_client();

    let adopted = adopt_seeds::advance(&mut home.conn, &key(), AT).expect("advance");
    assert_eq!(adopted, 1);

    let rows = replica_outbox::read(&home.conn, Scope::Entity("memory", &id), 1).expect("read");
    assert_eq!(rows.len(), 1, "the seed is now a pending outbox row");
    assert_eq!(rows[0].state, "pending");
}

#[test]
fn adopting_twice_queues_nothing_twice() {
    let mut home = Home::new();
    seed_one_pre_client(&mut home);
    home.make_client();

    let first = adopt_seeds::advance(&mut home.conn, &key(), AT).expect("first pass");
    let second = adopt_seeds::advance(&mut home.conn, &key(), AT).expect("second pass");
    assert_eq!(first, 1);
    assert_eq!(
        second, 0,
        "a cursor already at the head adopts nothing twice"
    );
}

#[test]
fn a_bound_entity_is_never_adopted() {
    let mut home = Home::new();
    let id = seed_one_pre_client(&mut home);
    home.make_client();
    let key = key();
    replica_binding::upsert(
        &home.conn,
        &key,
        &Binding {
            entity_kind: "memory".to_string(),
            entity_key: id.clone(),
            synced_digest: None,
            synced_deleted: false,
            synced_sequence: Some(1),
            synced_epoch: Some(home.epoch()),
        },
        AT,
    )
    .expect("bind");

    let adopted = adopt_seeds::advance(&mut home.conn, &key, AT).expect("advance");
    assert_eq!(
        adopted, 0,
        "an entity this key already holds is not re-sent"
    );
    assert!(
        replica_outbox::read(&home.conn, Scope::Entity("memory", &id), 1)
            .expect("read")
            .is_empty()
    );
}

#[test]
fn a_later_local_edit_carries_the_entity_and_the_seed_is_never_adopted() {
    let mut home = Home::new();
    let id = seed_one_pre_client(&mut home);
    home.make_client();
    home.retag(&id, &["retagged-before-adoption"]);

    let adopted = adopt_seeds::advance(&mut home.conn, &key(), AT).expect("advance");
    assert_eq!(
        adopted, 0,
        "the entity already has an outbox row from the retag; the older seed is not sent"
    );
    let rows = replica_outbox::read(&home.conn, Scope::Entity("memory", &id), 10).expect("read");
    assert_eq!(rows.len(), 1, "only the retag's own operation is pending");
}

#[test]
fn a_fresh_database_adopts_nothing() {
    let mut home = Home::new();
    home.make_client();
    let adopted = adopt_seeds::advance(&mut home.conn, &key(), AT).expect("advance");
    assert_eq!(adopted, 0);
}
