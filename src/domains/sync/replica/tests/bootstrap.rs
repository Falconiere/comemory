#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Seeding memories that predate the journal: restartable, idempotent, and
//! gating the capability until it is finished.

use crate::domains::sync::replica::{bootstrap, manifest};
use crate::store::replica_read;

use crate::domains::sync::replica::test_support as support;

use support::{BODY, Home};

/// Remove every journal row, leaving the memories — the state a database
/// upgraded from before the journal is in.
fn forget_the_journal(home: &Home) {
    home.conn
        .execute_batch(
            "DELETE FROM replica_feed; DELETE FROM replica_revision; \
             DELETE FROM replica_payload; DELETE FROM replica_operation; \
             DELETE FROM sqlite_sequence WHERE name = 'replica_feed'; \
             DELETE FROM schema_meta WHERE key LIKE 'replica_bootstrap%';",
        )
        .expect("clear journal");
}

#[test]
fn a_database_that_predates_the_journal_is_seeded_and_then_advertises() {
    let mut home = Home::new();
    home.save(BODY, &["sync"]);
    home.save(
        "A second memory, saved before the journal existed.",
        &["sync"],
    );
    forget_the_journal(&home);

    let mut ctx = home.ctx();
    let report = manifest::run(&mut ctx).expect("manifest");

    assert_eq!(
        report.capabilities,
        manifest::advertised(),
        "a scan that fits in one batch finishes before the manifest answers"
    );
    assert_eq!(report.bootstrap.state, "complete");
    assert_eq!(report.bootstrap.seeded, 2);
    assert_eq!(replica_read::head(&home.conn).expect("head"), 2);
}

#[test]
fn a_scan_longer_than_one_batch_hides_the_capability_until_it_finishes() {
    let mut home = Home::new();
    // One more than the 200-memory batch, so the first pass cannot finish.
    for n in 0..201 {
        home.save(
            &format!("Memory {n} saved before the replication journal existed."),
            &["sync"],
        );
    }
    forget_the_journal(&home);

    let mut ctx = home.ctx();
    let partway = manifest::run(&mut ctx).expect("manifest");
    assert_eq!(
        partway.capabilities,
        Vec::<String>::new(),
        "a half-seeded engine must not claim the protocol"
    );
    assert_eq!(partway.bootstrap.state, "seeding");
    assert_eq!(partway.bootstrap.seeded, 200);

    let mut ctx = home.ctx();
    let finished = manifest::run(&mut ctx).expect("manifest");
    assert_eq!(finished.capabilities, manifest::advertised());
    assert_eq!(finished.bootstrap.seeded, 201);
    assert_eq!(replica_read::head(&home.conn).expect("head"), 201);
}

#[test]
fn seeding_twice_adds_nothing_and_a_save_during_seeding_still_lands() {
    let mut home = Home::new();
    home.save(BODY, &["sync"]);
    forget_the_journal(&home);

    let mut ctx = home.ctx();
    bootstrap::advance(&mut ctx).expect("first pass");
    let after_first = replica_read::head(&home.conn).expect("head");

    // A concurrent save appends at the head like any other mutation.
    home.save("Saved while seeding was still running.", &["sync"]);
    let mut ctx = home.ctx();
    bootstrap::advance(&mut ctx).expect("second pass");
    let mut ctx = home.ctx();
    bootstrap::advance(&mut ctx).expect("third pass");

    let live: i64 = home
        .conn
        .query_row(
            "SELECT count(*) FROM replica_revision WHERE entity_kind = 'memory'",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(live, 2, "each memory has exactly one revision");
    assert!(
        replica_read::head(&home.conn).expect("head") > after_first,
        "the concurrent save is in the feed"
    );
}

#[test]
fn a_fresh_database_completes_seeding_with_nothing_to_do() {
    let mut home = Home::new();
    let mut ctx = home.ctx();
    let progress = bootstrap::advance(&mut ctx).expect("advance");
    assert!(progress.complete());
    assert_eq!(replica_read::head(&home.conn).expect("head"), 0);
}

#[test]
fn seed_without_enqueue_leaves_the_seeding_engine_owing_nothing() {
    let mut home = Home::new();
    home.save(BODY, &["sync"]);
    home.save(
        "A second memory, saved before the journal existed.",
        &["sync"],
    );
    forget_the_journal(&home);

    let mut ctx = home.ctx();
    bootstrap::advance(&mut ctx).expect("seed");

    assert_eq!(
        replica_read::head(&home.conn).expect("head"),
        2,
        "both were journalled"
    );
    assert_eq!(
        crate::store::replica_outbox::count(&home.conn, "pending").expect("owed"),
        0,
        "seeding records what this engine already holds; it owes no upload, and an owed \
         upload would make it refuse every import for those memories"
    );
}
