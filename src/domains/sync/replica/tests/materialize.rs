#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Materializing accepted memory operations against a real migrated database:
//! the feed keeps the operation id the sender minted, a tombstone replayed
//! after an interrupted delete still retires the mirror row, and the derived
//! graph refresh belongs to the caller, once per envelope.

use crate::domains::memories::MemoryStore;
use crate::domains::sync::replica::contract::Disposition;
use crate::domains::sync::replica::materialize::{self, Order};
use crate::domains::sync::replica::{accept, test_support as support};
use crate::store::replica_read;

use support::{Home, envelope, tombstone, upsert};

/// The operation id the feed recorded for `entity_key`'s newest position.
fn feed_operation(home: &Home, entity_key: &str) -> String {
    home.conn
        .query_row(
            "SELECT operation_id FROM replica_feed WHERE entity_key = ?1 \
             ORDER BY sequence DESC LIMIT 1",
            [entity_key],
            |r| r.get(0),
        )
        .expect("feed row")
}

#[test]
fn incoming_operation_id_is_the_feed_key_for_an_accepted_memory() {
    let mut sender = Home::new();
    let id = sender.save(support::BODY, &["sync"]);
    let payload = sender.payload(&id);
    let mut receiver = Home::new();
    let incoming = "op-20260924-0123456789abcdef0123456789abcdef";

    let response = {
        let mut ctx = receiver.ctx();
        accept::run(&mut ctx, envelope(vec![upsert(incoming, &payload)])).expect("accept")
    };

    assert_eq!(response.results[0].disposition, Disposition::Accepted);
    assert_eq!(
        feed_operation(&receiver, &id),
        incoming,
        "the receiver's feed names the operation the sender minted, so the sender can \
         recognize its own write when the feed hands it back"
    );
}

#[test]
fn tombstone_replay_after_the_markdown_moved_still_retires_the_row() {
    let mut home = Home::new();
    let id = home.save(support::BODY, &["sync"]);
    // The save's own upload is owed; answer it as a push would, so this case
    // is about the tombstone replay alone.
    support::mark_pushed(&home.conn, &id);
    // The state a kill between the markdown move and the commit leaves: the
    // file is already in the trash, the mirror row is still live. Produced
    // with the same library call the materializer makes.
    MemoryStore::new(home.paths.clone())
        .delete(&id)
        .expect("markdown moved to the trash");
    let live_before: Option<String> = home
        .conn
        .query_row(
            "SELECT deleted_at FROM memories WHERE id = ?1",
            [&id],
            |r| r.get(0),
        )
        .expect("row");
    assert!(
        live_before.is_none(),
        "the precondition: the row is still live"
    );

    let response = {
        let mut ctx = home.ctx();
        accept::run(
            &mut ctx,
            envelope(vec![tombstone(
                "op-20260924-replayedtombstone0000000000000001",
                &id,
            )]),
        )
        .expect("replay the tombstone")
    };

    assert_eq!(response.results[0].disposition, Disposition::Accepted);
    let deleted_at: Option<String> = home
        .conn
        .query_row(
            "SELECT deleted_at FROM memories WHERE id = ?1",
            [&id],
            |r| r.get(0),
        )
        .expect("row");
    assert!(
        deleted_at.is_some(),
        "a replayed tombstone leaves no live mirror row behind"
    );
}

#[test]
fn no_refresh_inside_materialize_and_one_per_envelope() {
    let mut sender = Home::new();
    let first = sender.save(support::BODY, &["sync"]);
    let second = sender.save(
        "comemory links a memory to the code it describes.",
        &["sync"],
    );
    let mut receiver = Home::new();
    let epoch = receiver.epoch();

    {
        let mut ctx = receiver.ctx();
        materialize::apply(
            &mut ctx,
            &epoch,
            &upsert(
                "op-20260924-norefresh00000000000000000000001",
                &sender.payload(&first),
            ),
            Order::Upstream,
        )
        .expect("materialize");
    }
    let rank: f64 = receiver
        .conn
        .query_row(
            "SELECT rank_score FROM memories WHERE id = ?1",
            [&first],
            |r| r.get(0),
        )
        .expect("rank");
    assert_eq!(
        rank, 0.0,
        "materialize alone leaves the derived graph for its caller"
    );

    {
        let mut ctx = receiver.ctx();
        accept::run(
            &mut ctx,
            envelope(vec![upsert(
                "op-20260924-norefresh00000000000000000000002",
                &sender.payload(&second),
            )]),
        )
        .expect("import");
    }
    let refreshed: f64 = receiver
        .conn
        .query_row(
            "SELECT rank_score FROM memories WHERE id = ?1",
            [&first],
            |r| r.get(0),
        )
        .expect("rank");
    assert!(
        refreshed > 0.0,
        "the import route refreshes the derived graph once for its envelope"
    );
    assert_eq!(replica_read::head(&receiver.conn).expect("head"), 2);
}
