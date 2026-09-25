#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Applying pulled entries against real migrated databases: entries are read
//! from a real sender's `changes` core, exactly what its route serves, and
//! applied on a second real data directory in the sender's order.

use crate::domains::learning::feedback_tracking::record_with_provenance;
use crate::domains::learning::telemetry::StatsDb;
use crate::domains::memories;
use crate::domains::sync::replica::contract::Disposition;
use crate::domains::sync::replica::contract_views::ChangeEntry;
use crate::domains::sync::replica::pulled::{self, Pulled};
use crate::domains::sync::replica::{changes, test_support as support};
use crate::utilities::telemetry::PROV_MANUAL;

use support::{BODY, Home};

/// Every entry the sender's feed holds, oldest first.
fn feed_of(sender: &mut Home) -> Vec<ChangeEntry> {
    let mut ctx = sender.ctx();
    changes::run(&mut ctx, 0, 500, None, None)
        .expect("changes")
        .entries
}

fn receipts(home: &Home, operation_id: &str) -> i64 {
    home.conn
        .query_row(
            "SELECT COUNT(*) FROM replica_receipt WHERE operation_id = ?1",
            [operation_id],
            |r| r.get(0),
        )
        .expect("count")
}

#[test]
fn a_pulled_memory_lands_under_its_upstream_id_and_replays_as_duplicate() {
    let mut sender = Home::new();
    let id = sender.save(BODY, &["sync"]);
    let entry = feed_of(&mut sender).remove(0);
    let mut receiver = Home::new();

    let first = {
        let mut ctx = receiver.ctx();
        pulled::apply(&mut ctx, &entry, pulled::Landing::Received).expect("apply")
    };
    let again = {
        let mut ctx = receiver.ctx();
        pulled::apply(&mut ctx, &entry, pulled::Landing::Received).expect("replay")
    };

    assert_eq!(first, Pulled::Applied);
    assert_eq!(
        again,
        Pulled::Duplicate,
        "a replay is answered by its receipt"
    );
    let (op_id, origin): (String, String) = receiver
        .conn
        .query_row(
            "SELECT operation_id, origin FROM replica_feed WHERE entity_key = ?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("feed row");
    assert_eq!(op_id, entry.operation_id);
    assert_eq!(
        origin, "sync",
        "a pulled change is never queued to be pushed back"
    );
    assert_eq!(
        crate::store::replica_outbox::count(&receiver.conn, "pending").expect("owed"),
        0
    );
}

#[test]
fn a_delete_and_restore_follow_the_upstreams_order_not_local_positions() {
    let mut sender = Home::new();
    let id = sender.save(BODY, &["sync"]);
    {
        let mut ctx = sender.ctx();
        memories::delete::run(&mut ctx, &id).expect("delete");
    }
    {
        let mut ctx = sender.ctx();
        memories::restore::run(&mut ctx, &id).expect("restore");
    }
    let entries = feed_of(&mut sender);
    assert_eq!(entries.len(), 3, "upsert, tombstone, restore");

    // The receiver has an unrelated memory first, so its local sequences
    // differ from the sender's: the restore's observed position names the
    // sender's tombstone, which acceptance would refuse against local state.
    let mut receiver = Home::new();
    let other = receiver.save("A memory only the receiver holds.", &["local"]);
    support::mark_pushed(&receiver.conn, &other);
    for entry in &entries {
        let mut ctx = receiver.ctx();
        assert_eq!(
            pulled::apply(&mut ctx, entry, pulled::Landing::Received).expect("apply"),
            Pulled::Applied,
            "{:?} follows the upstream's order",
            entry.op
        );
    }
    let deleted_at: Option<String> = receiver
        .conn
        .query_row(
            "SELECT deleted_at FROM memories WHERE id = ?1",
            [&id],
            |r| r.get(0),
        )
        .expect("row");
    assert!(deleted_at.is_none(), "the restore left the memory live");
}

#[test]
fn a_refused_entry_leaves_no_receipt_so_it_can_succeed_later() {
    let mut sender = Home::new();
    sender.save(BODY, &["sync"]);
    let mut tampered = feed_of(&mut sender).remove(0);
    tampered.payload_digest = Some("0".repeat(64));
    let mut receiver = Home::new();

    let answer = {
        let mut ctx = receiver.ctx();
        pulled::apply(&mut ctx, &tampered, pulled::Landing::Received).expect("apply")
    };

    assert_eq!(answer, Pulled::Refused(Disposition::RejectedInvalid));
    assert_eq!(
        receipts(&receiver, &tampered.operation_id),
        0,
        "no receipt: once the cause is fixed the same id can still apply"
    );
}

#[test]
fn an_unreadable_kind_is_refused_as_unsupported() {
    let mut sender = Home::new();
    sender.save(BODY, &["sync"]);
    let mut newer = feed_of(&mut sender).remove(0);
    newer.entity_kind = "future_kind".to_string();
    let mut receiver = Home::new();

    let mut ctx = receiver.ctx();
    assert_eq!(
        pulled::apply(&mut ctx, &newer, pulled::Landing::Received).expect("apply"),
        Pulled::Refused(Disposition::RejectedUnsupported)
    );
}

#[test]
fn an_event_already_held_is_never_counted_again_under_another_id() {
    let mut author = Home::new();
    support::approve(&author.conn, "Falconiere/comemory");
    let id = author.save(BODY, &["sync"]);
    let mut db = StatsDb::open(author.paths.stats_db()).expect("stats");
    record_with_provenance(&mut db, "q-20260924-1a2b3c4d", &[id], &[], PROV_MANUAL)
        .expect("verdict");
    let feed = feed_of(&mut author);
    let mut receiver = Home::new();
    for entry in &feed {
        let mut ctx = receiver.ctx();
        pulled::apply(&mut ctx, entry, pulled::Landing::Received).expect("apply");
    }
    let mut relayed = feed
        .into_iter()
        .find(|e| e.entity_kind == "feedback_event")
        .expect("the verdict was journalled");
    // The same event offered again under an id no receipt answers, as a
    // second relay path would carry it.
    relayed.operation_id = "op-20260924-relayedverdict00000000000000001".to_string();

    let again = {
        let mut ctx = receiver.ctx();
        pulled::apply(&mut ctx, &relayed, pulled::Landing::Received).expect("relayed")
    };
    let rewritten = {
        let mut ctx = receiver.ctx();
        pulled::apply(&mut ctx, &relayed, pulled::Landing::Rewrite).expect("rewrite")
    };

    assert_eq!((again, rewritten), (Pulled::Duplicate, Pulled::Duplicate));
    let (events, used): (i64, i64) = receiver
        .conn
        .query_row(
            "SELECT (SELECT COUNT(*) FROM feedback_events), \
                    (SELECT COALESCE(SUM(used_count), 0) FROM feedback)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("counts");
    assert_eq!(
        (events, used),
        (1, 1),
        "counted once, whatever id carried it"
    );
    assert_eq!(receipts(&receiver, &relayed.operation_id), 0);
}
