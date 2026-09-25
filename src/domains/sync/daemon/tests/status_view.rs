#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`super::status_view`] against a real (or absent) coordinator.

use comemory::config::paths::Paths;
use comemory::domains::sync::daemon::status_view::{State, view};

#[test]
fn an_empty_directory_with_no_coordinator_reports_not_running() {
    let dir = tempfile::Builder::new()
        .prefix("cm")
        .tempdir_in("/tmp")
        .unwrap();
    let paths = Paths::new(dir.path());
    paths.ensure_dirs().unwrap();
    let report = view(&paths).unwrap();
    assert_eq!(report.state, State::NotRunning);
    assert!(report.daemon.is_none());
    assert!(report.version_matches.is_none());
}
