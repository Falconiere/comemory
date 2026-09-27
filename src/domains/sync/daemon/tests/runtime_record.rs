#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`crate::domains::sync::daemon::runtime_record`].

use std::os::unix::fs::PermissionsExt as _;

use comemory::config::paths::Paths;
use comemory::domains::sync::daemon::runtime_record::{
    RECORD_FILE, RuntimeRecord, read, remove_if_ours, write,
};

fn record(instance: &str) -> RuntimeRecord {
    RuntimeRecord {
        pid: 4242,
        instance: instance.into(),
        socket: "/tmp/x.sock".into(),
        version: "0.0.0".into(),
        binary: "/usr/bin/comemory".into(),
        started_at: "2026-09-25T10:00:00Z".into(),
        supervisor: "process".into(),
    }
}

#[test]
fn a_record_round_trips_owner_only() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    write(&paths, &record("one")).unwrap();
    assert_eq!(read(&paths), Some(record("one")));
    let mode = std::fs::metadata(dir.path().join(RECORD_FILE))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn a_stopping_coordinator_never_removes_its_successors_record() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    write(&paths, &record("successor")).unwrap();
    remove_if_ours(&paths, "predecessor").unwrap();
    assert_eq!(read(&paths).map(|r| r.instance), Some("successor".into()));
    remove_if_ours(&paths, "successor").unwrap();
    assert!(read(&paths).is_none());
    remove_if_ours(&paths, "successor").unwrap();
}

#[test]
fn a_garbled_record_reads_as_absent() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(RECORD_FILE), "{ not json").unwrap();
    assert!(read(&Paths::new(dir.path())).is_none());
}
