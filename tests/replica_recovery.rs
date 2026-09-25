#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Real legacy database upgrade and journal seeding (#256, B-1). Every case
//! drives the real pinned `v0.43.2` binary, then this build's real
//! `comemory serve`, over real files — no hand-fabricated table dumps.

#[path = "common/legacy_engine.rs"]
mod legacy_engine;

#[path = "common/replica_support.rs"]
mod replica_support;

#[path = "common/recovery_support.rs"]
mod recovery_support;

use std::collections::HashSet;

use replica_support::Engine;

const REPO: &str = "legacy-corpus";
const MEMORY_COUNT: usize = 450;
const TRASHED_COUNT: usize = 20;

#[test]
fn legacy_database_upgrades_and_seeds_once() {
    let home = tempfile::tempdir().expect("tempdir");
    let data_dir = home.path().join(".comemory");

    let corpus =
        recovery_support::build_legacy_corpus(&data_dir, REPO, MEMORY_COUNT, TRASHED_COUNT);
    let (live_id, live_body) = corpus.first_live();
    let live_id = live_id.to_string();
    let query = recovery_support::distinctive_query(live_body);
    let query_id = recovery_support::legacy_verdict_and_runs(&data_dir, REPO, &live_id, &query);

    // The legacy binary is done writing. Snapshot the un-upgraded bytes so
    // the later refusal can be proven to have changed nothing.
    let before_upgrade = std::fs::read(data_dir.join("comemory.db")).expect("read legacy db");

    // Upgrading this build's `comemory serve` migrates the directory and
    // takes its pre-migration snapshot before any migration runs.
    let hub = Engine::spawn_at(&data_dir, &[]);
    recovery_support::approve(&hub, &[(REPO, "Falconiere/legacy-corpus")]);

    let snapshots = recovery_support::pre_migration_snapshots(&data_dir);
    assert_eq!(
        snapshots.len(),
        1,
        "exactly one pre-migration snapshot after the first open: {snapshots:?}"
    );

    // v0.43.2 still opens the snapshot it would have written.
    let snapshot_check = home.path().join("legacy-snapshot-check");
    std::fs::create_dir_all(&snapshot_check).expect("create scratch dir");
    std::fs::copy(&snapshots[0], snapshot_check.join("comemory.db")).expect("copy snapshot");
    assert!(
        recovery_support::legacy_binary_opens(&snapshot_check),
        "v0.43.2 must still open its own pre-migration snapshot"
    );

    // Opening a second time must not write a second snapshot.
    let _stopped = hub.stop();
    let hub = Engine::spawn_at(&data_dir, &[]);
    let snapshots_again = recovery_support::pre_migration_snapshots(&data_dir);
    assert_eq!(
        snapshots_again.len(),
        1,
        "a second open must not take a second snapshot: {snapshots_again:?}"
    );

    // v0.43.2 refuses the upgraded database outright, and changes nothing.
    assert!(
        !recovery_support::legacy_binary_opens(&data_dir),
        "v0.43.2 must refuse the migrated database"
    );
    let after_refusal = std::fs::read(data_dir.join("comemory.db")).expect("read upgraded db");
    assert_eq!(
        before_upgrade, after_refusal,
        "a refused open must not touch the database bytes"
    );

    // The manifest withholds capabilities until seeding finishes, then
    // advertises them.
    recovery_support::wait_for_capabilities(&hub);

    // One upsert per live memory, one tombstone per trashed memory, none
    // seeded twice.
    let entities = recovery_support::feed_entities_with_op(&data_dir);
    let memory: Vec<&(String, String, String)> = entities
        .iter()
        .filter(|(kind, ..)| kind == "memory")
        .collect();
    let expected_live = MEMORY_COUNT - TRASHED_COUNT;
    let upserts: Vec<&str> = memory
        .iter()
        .filter(|(_, _, op)| op == "upsert")
        .map(|(_, key, _)| key.as_str())
        .collect();
    let tombstones: Vec<&str> = memory
        .iter()
        .filter(|(_, _, op)| op == "tombstone")
        .map(|(_, key, _)| key.as_str())
        .collect();
    assert_eq!(
        upserts.len(),
        expected_live,
        "one feed position per live memory"
    );
    assert_eq!(
        tombstones.len(),
        TRASHED_COUNT,
        "one tombstone per trashed memory"
    );
    let unique_upserts: HashSet<&str> = upserts.iter().copied().collect();
    assert_eq!(
        unique_upserts.len(),
        expected_live,
        "every live memory seeded exactly once"
    );
    let unique_tombstones: HashSet<&str> = tombstones.iter().copied().collect();
    assert_eq!(
        unique_tombstones.len(),
        TRASHED_COUNT,
        "every trashed memory tombstoned exactly once"
    );
    assert!(
        unique_upserts.is_disjoint(&unique_tombstones),
        "a live id and a trashed id are never the same memory"
    );

    // One position per retained verdict and run.
    let (verdict, event_id) = recovery_support::feedback_event_row(&data_dir, &live_id, &query_id)
        .expect("the legacy verdict was retained and journalled");
    assert_eq!(verdict, "used");
    assert!(
        event_id.is_some(),
        "the retained verdict carries a stable replica event id"
    );
    assert!(
        recovery_support::shared_activity_count(&data_dir) >= 2,
        "the retained find/feedback runs were journalled as activity events"
    );
}
