#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Phase 1 of a compacting replay against a REAL engine: every position is
//! read, but only each entity's LAST entry is kept, and a kind-scoped scan
//! crosses a page the filter emptied by the raw continuation.

use crate::domains::sync::drain::pull::{Pull, Step};
use crate::domains::sync::drain::rebootstrap;
use crate::domains::sync::drain::replay_scan::{Scanned, page};
use crate::domains::sync::drain::test_support::{LiveEngine, approve, deliver, history_of_two};
use crate::domains::sync::replica::test_support::Home;
use crate::store::replica_replay;
use crate::store::sync_exchange::ExchangeRow;

#[test]
fn the_scan_keeps_only_the_last_entry_per_entity() {
    let engine = LiveEngine::start();
    let (m, n) = history_of_two(&engine);
    let reader = Home::new();
    let key = engine.key();
    let policy = approve(&reader.conn, &key, &["falconiere/comemory"]);
    let transport = engine.transport();
    let pull = Pull {
        key: &key,
        transport: &transport,
        policy: &policy,
        managed_revision: None,
    };
    let step = Step {
        paths: &reader.paths,
        cfg: &reader.cfg,
        pull: &pull,
        at: "t",
    };
    let epoch = engine.changes(0, None).stream_epoch;
    let mut row = ExchangeRow::fresh(&key);
    rebootstrap::start(&mut row, rebootstrap::REBOOTSTRAP, 3);

    assert_eq!(
        page(&reader.conn, &step, &mut row, &epoch, None).expect("scan"),
        Scanned::Done
    );

    let kept: Vec<(String, i64)> = replica_replay::next(&reader.conn, &key, 10)
        .expect("scratch")
        .into_iter()
        .map(|r| (r.entity_key, r.sequence))
        .collect();
    // m was saved, re-tagged, and n saved in between or after: whatever the
    // feed's order, scratch holds one row per entity, at its last position.
    let feed = engine.changes(0, None).entries;
    let last = |entity: &str| {
        feed.iter()
            .filter(|e| e.entity_key == entity)
            .map(|e| e.sequence)
            .max()
            .expect("a position")
    };
    let mut expected = vec![(m.clone(), last(&m)), (n.clone(), last(&n))];
    expected.sort_by_key(|(_, at)| *at);
    assert_eq!(
        kept, expected,
        "m's earlier position was superseded in scratch"
    );
    assert_eq!(feed.len(), 3, "three positions were read to keep two");
    assert_eq!(
        row.replay_scan_through,
        Some(3),
        "the scan resumes from here"
    );

    // A kind the feed does not hold: nothing is kept, and the empty filtered
    // page still carries the scan to the target.
    replica_replay::clear(&reader.conn, &key, None).expect("clear");
    let mut narrowed = ExchangeRow::fresh(&key);
    rebootstrap::start(&mut narrowed, "repair:document_revision", 3);
    let scanned = page(
        &reader.conn,
        &step,
        &mut narrowed,
        &epoch,
        Some("document_revision"),
    )
    .expect("scan");
    assert_eq!(scanned, Scanned::Done);
    assert_eq!(narrowed.replay_scan_through, Some(3));
    assert!(
        replica_replay::next(&reader.conn, &key, 10)
            .expect("scratch")
            .is_empty()
    );
}

/// Rules 1 and 5 at scanned positions settle with the dispositions the
/// forward pull records: this client's own operation `accepted`, the same
/// bytes under another id `already_upstream`.
#[test]
fn the_scan_settles_own_and_already_upstream_operations_like_the_pull() {
    let engine = LiveEngine::start();
    let mut reader = Home::new();
    let own = reader.save("A memory whose own operation the scan meets.", &[]);
    let copied = reader.save(
        "A memory the old protocol delivered as the same bytes.",
        &[],
    );
    deliver(&engine, &reader, &own, None);
    deliver(
        &engine,
        &reader,
        &copied,
        Some("op-20260924-0000000000000000000000000legacy2"),
    );
    let key = engine.key();
    let policy = approve(&reader.conn, &key, &["falconiere/comemory"]);
    let transport = engine.transport();
    let pull = Pull {
        key: &key,
        transport: &transport,
        policy: &policy,
        managed_revision: None,
    };
    let step = Step {
        paths: &reader.paths,
        cfg: &reader.cfg,
        pull: &pull,
        at: "t",
    };
    let epoch = engine.changes(0, None).stream_epoch;
    let mut row = ExchangeRow::fresh(&key);
    rebootstrap::start(&mut row, rebootstrap::REBOOTSTRAP, 2);

    page(&reader.conn, &step, &mut row, &epoch, None).expect("scan");

    let dispositions: Vec<(String, String, String)> = reader
        .conn
        .prepare("SELECT entity_key, state, disposition FROM replica_operation ORDER BY rowid")
        .expect("prepare")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect");
    assert_eq!(
        dispositions,
        vec![
            (own, "accepted".to_string(), "accepted".to_string()),
            (
                copied,
                "accepted".to_string(),
                "already_upstream".to_string()
            ),
        ]
    );
}
