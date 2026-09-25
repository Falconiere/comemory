#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Second mirror file for `src/domains/maintenance/rebuild.rs` (#256, B-2):
//! the memory bootstrap scan's `schema_meta` state through a real rebuild —
//! kept `complete` when the copy carries the journal whole, reset to
//! `pending` when it could not (a memory the old database never
//! journalled), so the manifest never falsely advertises capabilities.

use crate::test_common::cli_rebuild_support as support;

use comemory::config::{Config, Paths};
use comemory::utilities::context::Ctx;
use rusqlite::Connection;
use tempfile::{TempDir, tempdir};

use support::{count, open_db, run_save};

/// Call `maintenance::rebuild::run` over `home`'s data dir with a `Ctx::lazy`.
fn run_rebuild_api(home: &TempDir) -> comemory::prelude::Result<()> {
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    comemory::domains::maintenance::rebuild::run(
        &mut ctx,
        comemory::domains::maintenance::rebuild::Request {},
    )
}

// ---------------------------------------------------------------------------
// #256 B-2: a rebuild that copies replica state whole must keep advertising
// capabilities; one that could not fully carry the journal (a memory the old
// database never journalled) must not falsely report "complete".
// ---------------------------------------------------------------------------

fn bootstrap_state(conn: &Connection) -> Option<String> {
    conn.query_row(
        "SELECT value FROM schema_meta WHERE key = 'replica_bootstrap_state'",
        [],
        |r| r.get(0),
    )
    .ok()
}

/// Complete the memory bootstrap scan by driving it directly — the same
/// core a manifest or changes call, or #256's client-side drain step, would
/// run; a plain `comemory save` never touches it.
fn finish_bootstrap_scan(home: &TempDir) {
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut conn = open_db(home);
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    comemory::domains::sync::replica::bootstrap::advance(&mut ctx).expect("advance");
}

#[test]
fn a_rebuild_of_a_finished_journal_keeps_advertising_capabilities() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "already fully journalled"]);
    finish_bootstrap_scan(&home);
    {
        let conn = open_db(&home);
        assert_eq!(
            bootstrap_state(&conn).as_deref(),
            Some("complete"),
            "the one live memory already had its own revision from save time, \
             so the scan sees nothing left to seed"
        );
    }

    run_rebuild_api(&home).expect("rebuild");

    let conn = open_db(&home);
    assert_eq!(
        bootstrap_state(&conn).as_deref(),
        Some("complete"),
        "the copy carried the journal whole; a rebuilt hub still advertises"
    );
}

#[test]
fn a_rebuild_with_an_unjournalled_memory_resets_the_bootstrap_scan() {
    let home = tempdir().expect("tempdir");
    run_save(
        &home,
        &["--kind", "note", "will lose its journal row before rebuild"],
    );
    finish_bootstrap_scan(&home);
    {
        let conn = open_db(&home);
        assert_eq!(bootstrap_state(&conn).as_deref(), Some("complete"));
        conn.execute_batch(
            "DELETE FROM replica_feed; DELETE FROM replica_revision; \
             DELETE FROM replica_payload; DELETE FROM replica_operation; \
             DELETE FROM sqlite_sequence WHERE name = 'replica_feed';",
        )
        .expect("simulate an old database that never journalled this memory");
        assert_eq!(
            count(&conn, "SELECT count(*) FROM replica_revision"),
            0,
            "no revision for the live memory: exactly the unjournalled shape"
        );
    }

    run_rebuild_api(&home).expect("rebuild");

    let conn = open_db(&home);
    assert_eq!(
        bootstrap_state(&conn).as_deref(),
        Some("pending"),
        "the copy could not carry a journal row that never existed; the scan \
         must restart so the manifest withholds capabilities until it is \
         seeded once, rather than falsely reporting complete"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM memories WHERE deleted_at IS NULL"
        ),
        1,
        "the memory itself survived the replay"
    );
}
