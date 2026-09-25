#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The pull entry classifier over REAL engine response bodies: a live engine
//! is fed real saves, its `changes` page is read over HTTP, and each rule is
//! reached by the local state that triggers it.

use crate::domains::sync::drain::pull_rules::{Rule, classify};
use crate::domains::sync::drain::test_support::{LiveEngine, approve, deliver, push_all};
use crate::domains::sync::replica::contract_views::{ChangeEntry, PayloadState};
use crate::domains::sync::replica::test_support::{BODY, Home};
use crate::store::replica_binding::{self, Binding};
use crate::store::replica_outbox::{self, Scope};

const REPO: &str = "falconiere/comemory";

/// The entry the engine's feed holds for `entity_key` (its last one).
fn entry_for(engine: &LiveEngine, entity_key: &str) -> ChangeEntry {
    engine
        .changes(0, None)
        .entries
        .into_iter()
        .rev()
        .find(|e| e.entity_key == entity_key)
        .unwrap_or_else(|| panic!("no feed entry for {entity_key}"))
}

#[test]
fn every_rule_is_reached_by_the_local_state_that_triggers_it() {
    let engine = LiveEngine::start();
    let mut writer = Home::new();
    let shared = writer.save(BODY, &["sync"]);
    let peer_only = writer.save("A memory only the writer ever made.", &["sync"]);
    push_all(&writer, &engine);

    let mut reader = Home::new();
    let key = engine.key();
    let approved = approve(&reader.conn, &key, &[REPO]);
    let entry = entry_for(&engine, &shared);
    let epoch = engine.changes(0, None).stream_epoch;
    let rule = |home: &Home, policy, entry: &ChangeEntry| {
        classify(&home.conn, &key, policy, &epoch, entry).expect("classify")
    };

    // 8: nothing local stands in the way.
    assert_eq!(rule(&reader, &approved, &entry), Rule::Apply);

    // 2: a kind or version this build cannot read — before anything else.
    let mut newer = entry.clone();
    newer.schema_version = 9;
    assert_eq!(
        rule(&reader, &approved, &newer),
        Rule::Unreadable("memory@9".into())
    );

    // 4: the snapshot approves nothing.
    let nothing = approve(&reader.conn, &key, &[]);
    assert!(matches!(
        rule(&reader, &nothing, &entry),
        Rule::Unapproved(_)
    ));
    let approved = approve(&reader.conn, &key, &[REPO]);

    // 7: an upsert whose payload the upstream no longer holds.
    let mut erased = entry.clone();
    erased.payload_state = PayloadState::Erased;
    assert_eq!(rule(&reader, &approved, &erased), Rule::PayloadGone);

    // 6: this machine owes a different change to the same memory.
    reader.save("A memory only the writer ever made.", &["local"]);
    let peer_entry = entry_for(&engine, &peer_only);
    assert_eq!(rule(&reader, &approved, &peer_entry), Rule::PendingLocal);

    // 5: the upstream already holds the exact bytes a pending operation
    // carries, delivered under another id (the old protocol's way).
    let mine = reader.save(
        "A memory the reader made, delivered by the old protocol.",
        &[],
    );
    let owed = replica_outbox::read(&reader.conn, Scope::Entity("memory", &mine), 1)
        .expect("owed")
        .remove(0);
    deliver(
        &engine,
        &reader,
        &mine,
        Some("op-20260924-0000000000000000000000000legacy1"),
    );
    assert_eq!(
        rule(&reader, &approved, &entry_for(&engine, &mine)),
        Rule::AlreadyUpstream(vec![owed.operation_id.clone()])
    );

    // 1: the entry is this client's own operation.
    let own = reader.save("A memory the reader pushed under its own id.", &[]);
    deliver(&engine, &reader, &own, None);
    assert_eq!(
        rule(&reader, &approved, &entry_for(&engine, &own)),
        Rule::Own
    );

    // 3: older than what this key already applied for the entity.
    replica_binding::upsert(
        &reader.conn,
        &key,
        &Binding {
            entity_kind: "memory".into(),
            entity_key: shared.clone(),
            synced_digest: entry.payload_digest.clone(),
            synced_deleted: false,
            synced_sequence: Some(entry.sequence),
            synced_epoch: Some(epoch.clone()),
        },
        "t",
    )
    .expect("bind");
    assert_eq!(rule(&reader, &approved, &entry), Rule::Superseded);
}
