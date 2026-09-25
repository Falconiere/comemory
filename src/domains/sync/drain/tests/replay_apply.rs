#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Phase 2 of a compacting replay against a REAL engine: each entity's last
//! entry is applied in position order, the entity is bound at that position,
//! and its scratch row is deleted as it resolves.

use crate::domains::sync::drain::pull::{Pull, Step};
use crate::domains::sync::drain::rebootstrap;
use crate::domains::sync::drain::replay_apply::{Applied, batch};
use crate::domains::sync::drain::replay_scan;
use crate::domains::sync::drain::test_support::{LiveEngine, approve, history_of_two};
use crate::domains::sync::replica::test_support::Home;
use crate::store::replica_binding;
use crate::store::replica_replay;
use crate::store::sync_exchange::ExchangeRow;

#[test]
fn the_last_entries_apply_in_order_and_bind_at_their_positions() {
    let engine = LiveEngine::start();
    let (m, n) = history_of_two(&engine);
    let mut reader = Home::new();
    let key = engine.key();
    let policy = approve(&reader.conn, &key, &["falconiere/comemory"]);
    let transport = engine.transport();
    let pull = Pull {
        key: &key,
        transport: &transport,
        policy: &policy,
        managed_revision: None,
    };
    let (paths, cfg) = (reader.paths.clone(), reader.cfg.clone());
    let step = Step {
        paths: &paths,
        cfg: &cfg,
        pull: &pull,
        at: "t",
    };
    let epoch = engine.changes(0, None).stream_epoch;
    let mut row = ExchangeRow::fresh(&key);
    rebootstrap::start(&mut row, rebootstrap::REBOOTSTRAP, 3);
    replay_scan::page(&reader.conn, &step, &mut row, &epoch, None).expect("scan");

    assert_eq!(
        batch(&mut reader.conn, &step, &epoch).expect("apply"),
        Applied::Done(2)
    );

    assert!(
        replica_replay::next(&reader.conn, &key, 10)
            .expect("scratch")
            .is_empty()
    );
    let tags = reader.payload(&m).tags;
    assert_eq!(
        tags,
        vec!["second".to_string()],
        "the newer revision, never the older"
    );
    let feed = engine.changes(0, None).entries;
    for entity in [&m, &n] {
        let last = feed
            .iter()
            .filter(|e| &e.entity_key == entity)
            .map(|e| e.sequence)
            .max();
        let bound = replica_binding::get(&reader.conn, &key, "memory", entity)
            .expect("binding")
            .expect("bound");
        assert_eq!(bound.synced_sequence, last, "{entity}");
    }
}
