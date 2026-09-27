#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`crate::store::readiness`]: the read-only probe the resident
//! coordinator uses tells an absent, current, behind and ahead database
//! apart, and never creates or migrates one.

use comemory::store::readiness::{StoreReadiness, probe};

fn migrated_db(dir: &std::path::Path) -> std::path::PathBuf {
    let db = dir.join("comemory.db");
    drop(comemory::store::connection::open(&db).expect("open"));
    db
}

fn mtime(path: &std::path::Path) -> std::time::SystemTime {
    std::fs::metadata(path)
        .expect("meta")
        .modified()
        .expect("mtime")
}

#[test]
fn an_absent_database_is_absent_and_stays_absent() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("comemory.db");
    assert_eq!(probe(&db).expect("probe"), StoreReadiness::Absent);
    assert!(!db.exists(), "the probe must not create the corpus");
}

#[test]
fn a_fully_migrated_database_is_ready() {
    let dir = tempfile::tempdir().unwrap();
    let db = migrated_db(dir.path());
    assert_eq!(probe(&db).expect("probe"), StoreReadiness::Ready);
}

#[test]
fn a_database_missing_one_marker_awaits_migration_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let db = migrated_db(dir.path());
    let conn = rusqlite::Connection::open(&db).unwrap();
    let removed = conn
        .execute(
            "DELETE FROM schema_meta WHERE key = (SELECT key FROM schema_meta \
             WHERE key GLOB '[0-9][0-9][0-9][0-9]_*' ORDER BY key DESC LIMIT 1)",
            [],
        )
        .unwrap();
    assert_eq!(removed, 1);
    drop(conn);
    let before = mtime(&db);

    assert_eq!(probe(&db).expect("probe"), StoreReadiness::MigrationPending);
    assert_eq!(mtime(&db), before, "a pending migration is not applied");
    let conn = rusqlite::Connection::open(&db).unwrap();
    let markers: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM schema_meta WHERE key GLOB '[0-9][0-9][0-9][0-9]_*'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let expected = comemory::store::readiness::expected_marker_count();
    assert_eq!(usize::try_from(markers).unwrap() + 1, expected);
}

#[test]
fn a_database_from_a_newer_build_is_too_new() {
    let dir = tempfile::tempdir().unwrap();
    let db = migrated_db(dir.path());
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute(
        "INSERT INTO schema_meta (key, value) VALUES ('9999_from_the_future', '1')",
        [],
    )
    .unwrap();
    drop(conn);
    assert_eq!(probe(&db).expect("probe"), StoreReadiness::TooNew);
}

#[test]
fn an_empty_file_awaits_migration() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("comemory.db");
    std::fs::write(&db, b"").unwrap();
    assert_eq!(probe(&db).expect("probe"), StoreReadiness::MigrationPending);
}

#[test]
fn a_file_that_is_not_a_database_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("comemory.db");
    std::fs::write(
        &db,
        b"this is not sqlite, just some text of reasonable length",
    )
    .unwrap();
    assert!(probe(&db).is_err());
}
