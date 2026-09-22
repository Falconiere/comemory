#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! What the manifest reports, and when it is willing to say `replica-v1`.

use crate::domains::sync::replica::contract::PROTOCOL;
use crate::domains::sync::replica::{accept, manifest};

use crate::domains::sync::replica::test_support as support;

use support::{BODY, Home, envelope, tombstone};

#[test]
fn a_seeded_engine_advertises_the_protocol_and_reports_its_holdings() {
    let mut home = Home::new();
    home.save(BODY, &["sync"]);
    let mut ctx = home.ctx();
    let report = manifest::run(&mut ctx).expect("manifest");

    assert_eq!(report.protocol, PROTOCOL);
    assert_eq!(report.capabilities, vec![PROTOCOL.to_string()]);
    assert_eq!(report.bootstrap.state, "complete");
    assert_eq!(report.head_sequence, 1);
    let memories = report
        .entity_kinds
        .iter()
        .find(|k| k.kind == "memory")
        .expect("memory kind");
    assert_eq!(memories.count, 1);
    assert_eq!(memories.schema_version, 1);
    assert_eq!(memories.buckets.len(), 256);
    assert_eq!(report.bootstrap.seeded, 1);
    assert_eq!(home.epoch(), report.stream_epoch);
}

#[test]
fn two_engines_holding_the_same_memory_agree_on_every_bucket() {
    let mut author = Home::new();
    let id = author.save(BODY, &["sync"]);
    let payload = author.payload(&id);
    let mut peer = Home::new();
    let mut ctx = peer.ctx();
    accept::run(&mut ctx, envelope(vec![support::upsert("op-1", &payload)])).expect("import");

    let mut ctx = author.ctx();
    let mine = manifest::run(&mut ctx).expect("manifest");
    let mut ctx = peer.ctx();
    let theirs = manifest::run(&mut ctx).expect("manifest");

    let mine_buckets = &mine
        .entity_kinds
        .iter()
        .find(|k| k.kind == "memory")
        .expect("kind")
        .buckets;
    let their_buckets = &theirs
        .entity_kinds
        .iter()
        .find(|k| k.kind == "memory")
        .expect("kind")
        .buckets;
    assert_eq!(mine_buckets, their_buckets, "same holdings, same digest");
    assert_ne!(
        mine.stream_epoch, theirs.stream_epoch,
        "two databases are two streams"
    );
}

#[test]
fn a_deleted_memory_leaves_the_manifest() {
    let mut home = Home::new();
    let id = home.save(BODY, &["sync"]);
    let mut ctx = home.ctx();
    let before = manifest::run(&mut ctx).expect("manifest");
    assert_eq!(before.entity_kinds[0].count, 1);

    // The save's own operation must be off the outbox first: since #251 an
    // import is refused while this machine still owes a change to the same
    // memory, which is the guard that keeps a pull from overwriting an
    // unpushed local edit.
    support::mark_pushed(&home.conn, &id);
    let mut ctx = home.ctx();
    accept::run(&mut ctx, envelope(vec![tombstone("op-1", &id)])).expect("tombstone");
    let mut ctx = home.ctx();
    let after = manifest::run(&mut ctx).expect("manifest");

    assert_eq!(
        after.entity_kinds[0].count, 0,
        "a deleted memory holds nothing"
    );
    assert_ne!(
        after.entity_kinds[0].buckets, before.entity_kinds[0].buckets,
        "the manifest moved with the deletion"
    );
}

// ---------------------------------------------------------------------------
// #251: the manifest reports the embedding backlog, so an operator sees it
// without a second call.
// ---------------------------------------------------------------------------

#[test]
fn a_clean_engine_reports_no_embedding_backlog() {
    let mut home = Home::new();
    home.save(BODY, &["sync"]);
    let mut ctx = home.ctx();

    let report = manifest::run(&mut ctx).expect("manifest");

    assert_eq!(
        report.needs_embedding, 0,
        "a local save carries whatever vector it was given; nothing is owed"
    );
}

#[test]
fn an_import_whose_vector_was_refused_shows_up_in_the_manifest() {
    let mut author = Home::new();
    let id = author.save(BODY, &["sync"]);
    let payload = author.payload(&id);
    let mut peer = Home::new();
    let operation = support::upsert_with_vector(
        "op-20260922-refused1",
        &payload,
        support::wire_vector("text-embedding-3-small", 1024),
    );
    {
        let mut ctx = peer.ctx();
        accept::run(&mut ctx, envelope(vec![operation])).expect("accept");
    }

    let mut ctx = peer.ctx();
    let report = manifest::run(&mut ctx).expect("manifest");

    assert_eq!(
        report.needs_embedding, 1,
        "the memory replicated correctly and still cannot be found semantically"
    );
}
