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
use comemory::domains::sync::daemon::status_view::{State, view};

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
