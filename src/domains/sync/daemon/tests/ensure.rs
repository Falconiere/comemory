#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Pure-logic tests for this module's own `Intent`/`action_of`; the real
//! repair/replace behavior is proven against real coordinators in
//! `tests/replica_daemon.rs` (#257).

use super::{Intent, action_of};

#[test]
fn preflight_and_ensure_get_the_short_bound_repair_the_longer_one() {
    assert!(Intent::Preflight.bound() < Intent::Ensure.bound());
    assert_eq!(Intent::Ensure.bound(), Intent::Restart.bound());
    assert_eq!(Intent::Restart.bound(), Intent::Repair.bound());
}

#[test]
fn each_intent_names_a_distinct_action_except_preflight_and_ensure() {
    assert_eq!(action_of(Intent::Preflight), action_of(Intent::Ensure));
    assert_ne!(action_of(Intent::Restart), action_of(Intent::Repair));
    assert_ne!(action_of(Intent::Ensure), action_of(Intent::Restart));
}

// ---------------------------------------------------------------------------
// #258 D3c: preflight replaces a coordinator only when the caller is the
// file on disk at the coordinator's own path and the coordinator runs
// another file. Real files, real `stat` identities.
// ---------------------------------------------------------------------------

use std::path::{Path, PathBuf};

use super::preflight_replaces;
use crate::domains::sync::daemon::identity::{BinaryIdentity, file_id};
use crate::domains::sync::daemon::readiness::Readiness;

/// A real file at `dir/name` with its `<dev>:<ino>`.
fn real_file(dir: &Path, name: &str) -> (PathBuf, String) {
    let path = dir.join(name);
    std::fs::write(&path, name).unwrap();
    let id = file_id(&path).unwrap();
    (path, id)
}

fn readiness(binary: &Path, version: &str, binary_file: Option<&str>) -> Readiness {
    serde_json::from_value(serde_json::json!({
        "protocol": 1, "version": version, "binary": binary,
        "binary_file": binary_file, "pid": 1, "instance": "i",
        "started_at": "2026-09-27T00:00:00Z", "data_dir": "/d", "socket": "/d/s",
        "supervisor": "process", "store": "absent", "auth": {"state": "logged_out"},
        "sync": {"state": "idle", "interval_secs": 5, "passes": 0, "queued": false,
                 "channel": "off"}
    }))
    .unwrap()
}

fn caller(path: &Path, version: &str, file: &str) -> BinaryIdentity {
    BinaryIdentity {
        version: version.into(),
        path: path.to_path_buf(),
        file: Some(file.into()),
    }
}

#[test]
fn a_caller_that_is_the_new_file_replaces_the_coordinator_on_the_old_one() {
    let dir = tempfile::tempdir().unwrap();
    let (_, old) = real_file(dir.path(), "old");
    let (path, new) = real_file(dir.path(), "comemory");
    let running = readiness(&path, "0.50.0", Some(&old));
    assert!(preflight_replaces(
        &running,
        &caller(&path, "0.50.0", &new),
        Some(&new)
    ));
}

#[test]
fn an_older_process_still_running_the_replaced_file_never_evicts() {
    let dir = tempfile::tempdir().unwrap();
    let (_, old) = real_file(dir.path(), "old");
    let (path, new) = real_file(dir.path(), "comemory");
    // The coordinator already runs the new file; the caller is the old one.
    let running = readiness(&path, "0.50.0", Some(&new));
    assert!(!preflight_replaces(
        &running,
        &caller(&path, "0.50.0", &old),
        Some(&new)
    ));
}

#[test]
fn the_same_file_or_another_path_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let (path, id) = real_file(dir.path(), "comemory");
    let (other, other_id) = real_file(dir.path(), "copy");
    let me = caller(&path, "0.50.0", &id);
    assert!(!preflight_replaces(
        &readiness(&path, "0.50.0", Some(&id)),
        &me,
        Some(&id)
    ));
    assert!(!preflight_replaces(
        &readiness(&other, "0.50.0", Some(&other_id)),
        &me,
        Some(&id)
    ));
}

#[test]
fn a_coordinator_predating_binary_file_is_compared_by_version() {
    let dir = tempfile::tempdir().unwrap();
    let (path, id) = real_file(dir.path(), "comemory");
    let me = caller(&path, "0.50.1", &id);
    assert!(preflight_replaces(
        &readiness(&path, "0.50.0", None),
        &me,
        Some(&id)
    ));
    assert!(!preflight_replaces(
        &readiness(&path, "0.50.1", None),
        &me,
        Some(&id)
    ));
}
