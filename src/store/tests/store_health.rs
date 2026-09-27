#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Tests for [`crate::store::store_health`]: a migration that fails on a
//! long-lived open leaves a record the read-only probe reports as
//! `MigrationFailed`, a successful open clears it, and a data directory too
//! read-only to hold the record still reads as pending — never healthy.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use comemory::store::readiness::{StoreReadiness, probe};
use comemory::store::store_health::{self, Record};

/// A fully migrated database with its newest marker removed: behind by one
/// migration, which the next writable open applies.
fn one_migration_behind(dir: &Path) -> PathBuf {
    let db = dir.join("comemory.db");
    drop(comemory::store::connection::open(&db).expect("open"));
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute(
        "DELETE FROM schema_meta WHERE key = (SELECT key FROM schema_meta \
         WHERE key GLOB '[0-9][0-9][0-9][0-9]_*' ORDER BY key DESC LIMIT 1)",
        [],
    )
    .unwrap();
    drop(conn);
    db
}

fn sidecar(db: &Path, suffix: &str) -> PathBuf {
    let mut name = db.as_os_str().to_os_string();
    name.push(suffix);
    name.into()
}

fn set_mode(path: &Path, mode: u32) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
}

#[test]
fn a_failed_migration_is_recorded_until_an_open_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let db = one_migration_behind(dir.path());
    set_mode(&db, 0o444);

    let err = store_health::open_recorded(&db).expect_err("a read-only file cannot migrate");
    let record = store_health::failed_migration(&db).expect("the failure is recorded");
    assert_eq!(record.state, store_health::MIGRATION_FAILED);
    assert_eq!(record.binary_version, env!("CARGO_PKG_VERSION"));
    assert!(
        record.detail.contains(&err.to_string()),
        "{record:?} vs {err}"
    );
    assert_eq!(probe(&db).expect("probe"), StoreReadiness::MigrationFailed);

    // SQLite gave the `-wal`/`-shm` it created the database file's mode.
    for file in [db.clone(), sidecar(&db, "-wal"), sidecar(&db, "-shm")] {
        if file.exists() {
            set_mode(&file, 0o644);
        }
    }
    drop(store_health::open_recorded(&db).expect("the retried open migrates"));
    assert!(
        !store_health::record_path(&db).exists(),
        "success clears the record"
    );
    assert_eq!(probe(&db).expect("probe"), StoreReadiness::Ready);
}

#[test]
fn a_record_left_by_another_binary_reads_as_pending() {
    let dir = tempfile::tempdir().unwrap();
    let db = one_migration_behind(dir.path());
    let record = Record {
        state: store_health::MIGRATION_FAILED.into(),
        detail: "an older build failed".into(),
        at: "2026-09-27T00:00:00Z".into(),
        binary_version: "0.0.1".into(),
    };
    std::fs::write(
        store_health::record_path(&db),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();

    assert!(store_health::failed_migration(&db).is_none());
    assert_eq!(probe(&db).expect("probe"), StoreReadiness::MigrationPending);
}

#[test]
fn a_data_dir_too_read_only_for_the_record_still_reads_as_pending() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir(&data).unwrap();
    let db = one_migration_behind(&data);
    set_mode(&data, 0o555);

    let opened = store_health::open_recorded(&db);
    let pending = probe(&db);
    let recorded = store_health::record_path(&db).exists();
    set_mode(&data, 0o755);

    assert!(opened.is_err(), "a read-only data directory cannot migrate");
    assert!(!recorded, "nowhere to write the record");
    assert_eq!(pending.expect("probe"), StoreReadiness::MigrationPending);
}
