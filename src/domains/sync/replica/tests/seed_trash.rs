#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Seeding memories trashed before the journal existed: tombstones, not
//! upserts, restartable, and read from `.trash/` — not the mirror, which a
//! rebuild never repopulates for a trashed id.

use crate::domains::sync::replica::seed_trash;
use crate::store::replica_read;

use crate::domains::sync::replica::test_support as support;
use support::{Home, forget_the_journal};

#[test]
fn a_trashed_memory_is_journalled_as_a_tombstone() {
    let mut home = Home::new();
    let id = home.save(
        "A memory that will be trashed before the journal exists.",
        &["sync"],
    );
    home.delete(&id);
    forget_the_journal(&home);

    let mut ctx = home.ctx();
    let progress = seed_trash::advance(&mut ctx).expect("advance");
    assert!(progress.complete());

    let revision = replica_read::revision(&home.conn, "memory", &id)
        .expect("revision lookup")
        .expect("the trashed memory has a revision");
    assert!(revision.deleted, "seeded as a tombstone, not a live upsert");
    assert!(
        revision.payload_digest.is_none(),
        "a tombstone carries no payload"
    );
}

#[test]
fn seeding_twice_journals_no_second_tombstone() {
    let mut home = Home::new();
    let id = home.save("Trashed once, seeded twice.", &["sync"]);
    home.delete(&id);
    forget_the_journal(&home);

    let mut ctx = home.ctx();
    seed_trash::advance(&mut ctx).expect("first pass");
    let head_after_first = replica_read::head(&home.conn).expect("head");

    let mut ctx = home.ctx();
    seed_trash::advance(&mut ctx).expect("second pass");
    assert_eq!(
        replica_read::head(&home.conn).expect("head"),
        head_after_first,
        "an already-tombstoned id is skipped, not re-journalled"
    );
}

#[test]
fn a_lost_cursor_reseeds_nothing_once_the_journal_is_intact() {
    let mut home = Home::new();
    let id = home.save("Trashed and already fully seeded.", &["sync"]);
    home.delete(&id);
    forget_the_journal(&home);
    let mut ctx = home.ctx();
    seed_trash::advance(&mut ctx).expect("seed");
    let head_before = replica_read::head(&home.conn).expect("head");

    // A rebuild resets the cursor but keeps the journal row — the shape a
    // real `comemory rebuild` leaves (schema_meta is not copied verbatim in
    // this unit test; only the cursor keys are cleared here).
    home.conn
        .execute_batch("DELETE FROM schema_meta WHERE key LIKE 'replica_seed_trash%';")
        .expect("drop cursor");

    let mut ctx = home.ctx();
    let progress = seed_trash::advance(&mut ctx).expect("rescan");
    assert!(progress.complete());
    assert_eq!(
        replica_read::head(&home.conn).expect("head"),
        head_before,
        "the intact revision is skipped on the rescan; nothing is seeded twice"
    );
}

#[test]
fn a_live_memory_is_never_seeded_by_the_trash_walk() {
    let mut home = Home::new();
    let id = home.save("Never trashed.", &["sync"]);
    forget_the_journal(&home);

    let mut ctx = home.ctx();
    seed_trash::advance(&mut ctx).expect("advance");

    assert!(
        replica_read::revision(&home.conn, "memory", &id)
            .expect("lookup")
            .is_none(),
        "the live-memory walk owns this id, not the trash walk"
    );
}

#[test]
fn a_fresh_data_dir_completes_with_nothing_to_seed() {
    let mut home = Home::new();
    let mut ctx = home.ctx();
    let progress = seed_trash::advance(&mut ctx).expect("advance");
    assert!(progress.complete());
}
