#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `store::schema_journal` against copies of the SHIPPED `migrations/`
//! files: the journal gains exactly the entry `run_generate` would have
//! written, a duplicate name is refused before any write, and `adopt` is a
//! byte-identical no-op the second time — the guarantees `just
//! migration-journal` / `just migration-adopt` rest on.

use std::fs;
use std::path::{Path, PathBuf};

use comemory::errors::Error;
use comemory::store::schema_journal::{Journaled, adopt, journal_file};
use tempfile::tempdir;
use toolu_orm::core::journal::{Journal, compute_hash};

const SHIPPED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations");

/// A tempdir holding copies of the shipped journal and newest snapshot.
fn shipped_copy() -> (tempfile::TempDir, PathBuf) {
    let dir = tempdir().expect("tempdir");
    for name in ["_journal.json", "0016_v16_sync.snapshot.json"] {
        fs::copy(Path::new(SHIPPED).join(name), dir.path().join(name)).unwrap();
    }
    let path = dir.path().to_path_buf();
    (dir, path)
}

fn read_journal(dir: &Path) -> Journal {
    Journal::read_from_path(dir.join("_journal.json").to_str().unwrap()).unwrap()
}

#[test]
fn journal_file_appends_the_basename_and_the_hash_of_the_bytes() {
    let (_guard, dir) = shipped_copy();
    let sql = "CREATE TABLE probe (id INTEGER PRIMARY KEY);\n";
    let file = dir.join("0017_probe.sql");
    fs::write(&file, sql).unwrap();

    let journaled = journal_file(&dir, &file).expect("journal a new file");

    assert_eq!(
        journaled,
        Journaled {
            name: "0017_probe.sql".to_owned(),
            hash: compute_hash(sql),
        }
    );
    let journal = read_journal(&dir);
    assert_eq!(journal.entries.len(), 17);
    let last = journal.entries.last().unwrap();
    assert_eq!((last.idx, last.name.as_str()), (16, "0017_probe.sql"));
    assert_eq!(last.hash, compute_hash(sql));
}

#[test]
fn journal_file_refuses_a_name_already_journaled_and_writes_nothing() {
    let (_guard, dir) = shipped_copy();
    // The shipped file, copied beside the journal so the containment rule
    // passes and the duplicate-name rule is what fires.
    let copy = dir.join("0016_v16_sync.sql");
    fs::copy(Path::new(SHIPPED).join("0016_v16_sync.sql"), &copy).unwrap();
    let before = fs::read_to_string(dir.join("_journal.json")).unwrap();

    let err = journal_file(&dir, &copy).expect_err("a journaled name is refused");

    assert!(
        matches!(err, Error::Usage(ref m) if m == "0016_v16_sync.sql is already journaled"),
        "got {err:?}"
    );
    assert_eq!(
        fs::read_to_string(dir.join("_journal.json")).unwrap(),
        before,
        "the journal is untouched"
    );
}

#[test]
fn journal_file_reports_a_missing_migration() {
    let (_guard, dir) = shipped_copy();
    let err = journal_file(&dir, &dir.join("0099_missing.sql")).expect_err("missing file");
    assert!(
        matches!(err, Error::Other(ref m) if m.contains("0099_missing.sql")),
        "got {err:?}"
    );
}

#[test]
fn adopt_is_byte_identical_the_second_time_and_keeps_the_snapshot_identity() {
    let (_guard, dir) = shipped_copy();
    let snapshot = dir.join("0016_v16_sync.snapshot.json");
    let shipped = fs::read(&snapshot).unwrap();

    let first = adopt(&dir).expect("first adopt");
    assert_eq!(first, snapshot);
    let after_first = fs::read(&snapshot).unwrap();
    assert_eq!(
        after_first, shipped,
        "the shipped snapshot already describes the registry"
    );

    let second = adopt(&dir).expect("second adopt");
    assert_eq!(second, snapshot);
    assert_eq!(
        fs::read(&snapshot).unwrap(),
        after_first,
        "adopt is idempotent"
    );
}

#[test]
fn adopt_creates_the_snapshot_for_a_journal_that_has_none_yet() {
    let (_guard, dir) = shipped_copy();
    let snapshot = dir.join("0016_v16_sync.snapshot.json");
    fs::remove_file(&snapshot).unwrap();

    let written = adopt(&dir).expect("adopt without an existing snapshot");

    assert_eq!(written, snapshot);
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&snapshot).unwrap()).unwrap();
    assert_eq!(
        json["prev_id"].as_str(),
        Some("00000000-0000-0000-0000-000000000000"),
        "a snapshot created from nothing is a root"
    );
    let tables = json["tables"].as_object().expect("tables object");
    assert_eq!(
        tables.len(),
        comemory::store::schema::DECLARED_TABLES.len(),
        "every declared table is in it"
    );
    assert!(tables.contains_key("code_symbols"));
}

#[test]
fn journal_file_refuses_a_migration_outside_the_migrations_directory() {
    let (_guard, dir) = shipped_copy();
    let elsewhere = tempdir().expect("tempdir");
    let stray = elsewhere.path().join("0017_stray.sql");
    fs::write(&stray, "SELECT 1;\n").unwrap();
    let before = fs::read_to_string(dir.join("_journal.json")).unwrap();

    let err = journal_file(&dir, &stray).expect_err("a file outside dir is refused");

    assert!(
        matches!(err, Error::Usage(ref m) if m.contains("0017_stray.sql") && m.contains("not inside")),
        "got {err:?}"
    );
    assert_eq!(
        fs::read_to_string(dir.join("_journal.json")).unwrap(),
        before
    );
}

#[test]
fn adopt_refuses_an_empty_journal() {
    let dir = tempdir().expect("tempdir");
    let err = adopt(dir.path()).expect_err("no journal entries");
    assert!(matches!(err, Error::Usage(_)), "got {err:?}");
}
