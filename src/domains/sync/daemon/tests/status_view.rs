#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`super::status_view`] against a real (or absent) coordinator.
//!
//! Both tests read/clear `COMEMORY_SYNC_DAEMON`, so they are in the
//! `env-mutating` nextest group (`.config/nextest.toml`).

use comemory::config::paths::Paths;
use comemory::domains::sync::daemon::readiness::StoreState;
use comemory::domains::sync::daemon::status_view::{State, view};
use comemory::domains::sync::replica::restore_state;

fn empty_dir() -> (tempfile::TempDir, Paths) {
    let dir = tempfile::Builder::new()
        .prefix("cm")
        .tempdir_in("/tmp")
        .unwrap();
    let paths = Paths::new(dir.path());
    paths.ensure_dirs().unwrap();
    (dir, paths)
}

#[test]
fn an_empty_directory_with_no_coordinator_reports_not_running() {
    let (_dir, paths) = empty_dir();
    // SAFETY: this test is in the env-mutating nextest group (max-threads=1).
    // `.cargo/config.toml`'s hermetic default sets this to "0" for every
    // in-process test; clear it so `view` sees a real absent-coordinator case.
    unsafe { std::env::remove_var("COMEMORY_SYNC_DAEMON") };
    let report = view(&paths).unwrap();
    assert_eq!(report.state, State::NotRunning);
    assert!(report.daemon.is_none());
    assert!(report.version_matches.is_none());
}

#[test]
fn the_harness_switch_reports_disabled_without_probing() {
    let (_dir, paths) = empty_dir();
    // SAFETY: this test is in the env-mutating nextest group (max-threads=1).
    unsafe { std::env::set_var("COMEMORY_SYNC_DAEMON", "0") };
    let report = view(&paths).unwrap();
    assert_eq!(report.state, State::Disabled);
    assert!(report.daemon.is_none());
}

#[test]
fn the_store_is_reported_whatever_the_coordinator_state() {
    let (_dir, paths) = empty_dir();
    // SAFETY: env-mutating nextest group (max-threads=1), as above.
    unsafe { std::env::set_var("COMEMORY_SYNC_DAEMON", "0") };
    let absent = view(&paths).unwrap();
    assert_eq!(absent.state, State::Disabled);
    assert_eq!(absent.store, StoreState::Absent);
    assert!(absent.healthy, "no store yet needs no operator");
    assert!(!paths.db_path().exists(), "the probe creates nothing");

    let conn = comemory::store::connection::open(paths.db_path()).unwrap();
    assert_eq!(view(&paths).unwrap().store, StoreState::Ready);
    restore_state::set(&conn, restore_state::State::ErasureUnknown).unwrap();
    let unverified = view(&paths).unwrap();
    assert_eq!(unverified.store, StoreState::RestoreUnverified);
    assert!(!unverified.healthy);

    restore_state::clear(&conn).unwrap();
    std::fs::write(restore_state::pending_path(&paths), b"{}").unwrap();
    assert_eq!(view(&paths).unwrap().store, StoreState::RestoreUnverified);
}
