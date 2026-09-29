#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Migrations 0031 and 0032 over a real pre-projects data directory (#325,
//! #324): the pinned `v0.52.0` release binary writes it, the directory is
//! copied, and this build opens the copy. The copy reaches the current version
//! with its memory intact and every project table and the change feed present
//! and empty; the original is left at 30.

#[path = "common/legacy_engine.rs"]
mod legacy_engine;

use std::path::Path;

use assert_cmd::Command;
use comemory::store::schema_projects::PROJECT_TABLES;
use rusqlite::Connection;
use tempfile::tempdir;

fn schema_version(dir: &Path) -> String {
    Connection::open(dir.join("comemory.db"))
        .expect("open db")
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'version'",
            [],
            |r| r.get(0),
        )
        .expect("version")
}

/// Copy `from` into `to` recursively: the database, its sidecars and the
/// markdown the memory lives in.
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create copy");
    for entry in std::fs::read_dir(from).expect("read dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy file");
        }
    }
}

#[test]
fn a_v0_52_0_data_directory_upgrades_to_the_project_tables() {
    let legacy = tempdir().expect("legacy home");
    let saved = legacy_engine::run_json_of(
        &legacy_engine::V0_52_0,
        legacy.path(),
        &[
            "save",
            "Projects need an engine-side charter before any command core lands.",
            "--kind",
            "decision",
            "--repo",
            "Falconiere/comemory",
        ],
    );
    let memory_id = saved["id"].as_str().expect("saved id").to_string();
    legacy_engine::run_json_of(&legacy_engine::V0_52_0, legacy.path(), &["stats"]);
    assert_eq!(
        schema_version(legacy.path()),
        "30",
        "v0.52.0 writes schema 30"
    );

    let copy = tempdir().expect("copy home");
    copy_dir(legacy.path(), copy.path());

    let shown = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", copy.path())
        .env("HOME", copy.path())
        .args(["--json", "show", &memory_id])
        .output()
        .expect("run comemory show");
    assert!(
        shown.status.success(),
        "this build opens and upgrades the copy: {}",
        String::from_utf8_lossy(&shown.stderr)
    );
    let shown: serde_json::Value = serde_json::from_slice(&shown.stdout).expect("show json");
    assert_eq!(
        shown["id"],
        memory_id.as_str(),
        "the memory survives the upgrade"
    );

    assert_eq!(schema_version(copy.path()), "32");
    let conn = Connection::open(copy.path().join("comemory.db")).expect("open copy");
    for table in PROJECT_TABLES.iter().chain(&["project_changes"]) {
        let rows: i64 = conn
            .query_row(&format!("SELECT count(*) FROM \"{table}\""), [], |r| {
                r.get(0)
            })
            .unwrap_or_else(|e| panic!("{table} exists after the upgrade: {e}"));
        assert_eq!(rows, 0, "{table} starts empty");
    }
    assert_eq!(
        schema_version(legacy.path()),
        "30",
        "the original directory is untouched"
    );
}
