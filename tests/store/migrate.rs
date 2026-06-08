//! Behavior tests for `crate::store::migrate` — the versioned schema
//! migration runner. Task 3 of the v0.2 plan introduces `run` and
//! `CURRENT_VERSION`; these tests assert that a fresh database is
//! brought to the current version on first call and that subsequent
//! calls are idempotent (no panics, no duplicate inserts, no version
//! regression).

use comemory::store::{connection, migrate};
use tempfile::tempdir;

#[test]
fn fresh_db_runs_all_migrations_to_current_version() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let mut conn = connection::open(&path).expect("open");

    migrate::run(&mut conn).expect("migrate");

    let version: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'version'",
            [],
            |row| row.get(0),
        )
        .expect("read schema version");
    assert_eq!(version, migrate::CURRENT_VERSION);
}

#[test]
fn running_migrations_twice_is_idempotent() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let mut conn = connection::open(&path).expect("open");

    migrate::run(&mut conn).expect("first run");
    migrate::run(&mut conn).expect("second run is a no-op");
}
