#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! [`comemory::store::code_generation`] against a real migrated database —
//! the activation rule that decides which generation a repo is at, and the
//! two ways it refuses to produce a union of two heads.

use comemory::store::code_generation::{self, Generation, State};
use comemory::store::replica_journal::ReplicaOrigin;
use comemory::store::{connection, migrate};
use rusqlite::Connection;
use tempfile::TempDir;

/// The repo every case here is about.
const REPO: &str = "Falconiere/comemory";

fn migrated_db() -> (TempDir, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    migrate::run(&mut conn).expect("migrate");
    (dir, conn)
}

fn generation(id: &str, parent: Option<&str>, origin: ReplicaOrigin) -> Generation {
    Generation {
        repo: REPO.to_string(),
        generation_id: id.to_string(),
        parent_id: parent.map(str::to_string),
        head: format!("head-for-{id}"),
        mined_commit: Some("mined-1".to_string()),
        origin,
        state: State::Staged,
        file_count: 3,
        manifest_digest: format!("{id}-digest"),
    }
}

fn record(conn: &Connection, g: &Generation) {
    code_generation::record(conn, g, "2026-09-22T10:00:00Z").expect("record");
}

#[test]
fn a_staged_generation_is_not_the_repos_active_one() {
    let (_dir, conn) = migrated_db();
    record(&conn, &generation("gen1", None, ReplicaOrigin::Local));

    assert_eq!(code_generation::active(&conn, REPO).expect("active"), None);
    let stored = code_generation::by_id(&conn, REPO, "gen1")
        .expect("by_id")
        .expect("row");
    assert_eq!(stored.state, State::Staged);
    assert_eq!(stored.parent_id, None);
}

#[test]
fn activation_makes_one_generation_active_and_supersedes_the_one_it_replaces() {
    let (_dir, conn) = migrated_db();
    record(&conn, &generation("gen1", None, ReplicaOrigin::Local));
    code_generation::activate(&conn, REPO, "gen1", "2026-09-22T10:01:00Z").expect("activate one");
    record(
        &conn,
        &generation("gen2", Some("gen1"), ReplicaOrigin::Local),
    );

    code_generation::activate(&conn, REPO, "gen2", "2026-09-22T10:02:00Z").expect("activate two");

    let active = code_generation::active(&conn, REPO)
        .expect("active")
        .expect("row");
    assert_eq!(active.generation_id, "gen2");
    let prior = code_generation::by_id(&conn, REPO, "gen1")
        .expect("by_id")
        .expect("row");
    assert_eq!(
        prior.state,
        State::Superseded,
        "exactly one generation is active; the one it replaced is superseded"
    );
}

#[test]
fn activating_the_already_active_generation_is_a_no_op() {
    let (_dir, conn) = migrated_db();
    record(&conn, &generation("gen1", None, ReplicaOrigin::Local));
    code_generation::activate(&conn, REPO, "gen1", "2026-09-22T10:01:00Z").expect("activate");

    code_generation::activate(&conn, REPO, "gen1", "2026-09-22T10:05:00Z")
        .expect("a replayed acceptance costs a no-op, not a refusal");

    let active = code_generation::active(&conn, REPO)
        .expect("active")
        .expect("row");
    assert_eq!(active.generation_id, "gen1");
    assert_eq!(
        code_generation::all(&conn, REPO).expect("all").len(),
        1,
        "and no second row appeared"
    );
}

#[test]
fn a_generation_planned_against_a_superseded_parent_is_refused() {
    let (_dir, conn) = migrated_db();
    record(&conn, &generation("gen1", None, ReplicaOrigin::Local));
    code_generation::activate(&conn, REPO, "gen1", "2026-09-22T10:01:00Z").expect("activate one");
    record(
        &conn,
        &generation("gen2", Some("gen1"), ReplicaOrigin::Local),
    );
    code_generation::activate(&conn, REPO, "gen2", "2026-09-22T10:02:00Z").expect("activate two");
    // Planned against gen1 while the repo has already moved to gen2 — the
    // concurrent-generation case. Merging it would union two heads.
    record(
        &conn,
        &generation("gen3", Some("gen1"), ReplicaOrigin::Local),
    );

    let refused = code_generation::activate(&conn, REPO, "gen3", "2026-09-22T10:03:00Z")
        .expect_err("a stale plan must be refused so the caller replans");

    assert!(
        matches!(refused, comemory::errors::Error::Conflict(_)),
        "got {refused:?}"
    );
    let active = code_generation::active(&conn, REPO)
        .expect("active")
        .expect("row");
    assert_eq!(active.generation_id, "gen2", "the repo did not move");
}

#[test]
fn activating_a_generation_that_was_never_recorded_is_not_found() {
    let (_dir, conn) = migrated_db();

    let missing = code_generation::activate(&conn, REPO, "nosuch", "2026-09-22T10:00:00Z")
        .expect_err("nothing to activate");

    assert!(
        matches!(missing, comemory::errors::Error::NotFound(_)),
        "got {missing:?}"
    );
}

#[test]
fn only_a_locally_built_generation_is_offered_upstream() {
    let (_dir, conn) = migrated_db();
    record(&conn, &generation("pulled", None, ReplicaOrigin::Sync));
    code_generation::activate(&conn, REPO, "pulled", "2026-09-22T10:01:00Z").expect("activate");

    assert_eq!(
        code_generation::active(&conn, REPO)
            .expect("active")
            .map(|g| g.generation_id),
        Some("pulled".to_string()),
        "the pulled generation is what the repo is at"
    );
    assert_eq!(
        code_generation::active(&conn, REPO)
            .expect("active")
            .map(|g| g.origin),
        Some(ReplicaOrigin::Sync),
        "but it is never offered back — that is how a replication loop is prevented"
    );
}

#[test]
fn recording_the_same_generation_twice_updates_rather_than_duplicates() {
    let (_dir, conn) = migrated_db();
    record(&conn, &generation("gen1", None, ReplicaOrigin::Local));
    let mut moved = generation("gen1", None, ReplicaOrigin::Local);
    moved.head = "a-later-head".to_string();
    moved.file_count = 9;
    record(&conn, &moved);

    let all = code_generation::all(&conn, REPO).expect("all");
    assert_eq!(all.len(), 1, "one id, one row");
    assert_eq!(all[0].head, "a-later-head");
    assert_eq!(all[0].file_count, 9);
}

#[test]
fn generations_of_other_repos_are_not_this_repos_business() {
    let (_dir, conn) = migrated_db();
    record(&conn, &generation("gen1", None, ReplicaOrigin::Local));
    code_generation::activate(&conn, REPO, "gen1", "2026-09-22T10:01:00Z").expect("activate");

    assert_eq!(
        code_generation::active(&conn, "Falconiere/other").expect("active"),
        None
    );
    assert!(
        code_generation::all(&conn, "Falconiere/other")
            .expect("all")
            .is_empty()
    );
}

#[test]
fn the_database_refuses_a_state_outside_the_declared_vocabulary() {
    let (_dir, conn) = migrated_db();
    record(&conn, &generation("gen1", None, ReplicaOrigin::Local));

    conn.execute_batch(
        "UPDATE code_generation SET state = 'halfway' WHERE generation_id = 'gen1';",
    )
    .expect_err("the CHECK constraint refuses an unknown state");
}
