#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! [`super::spawn`] is exercised end to end (real detached process, real
//! log files) by `tests/replica_daemon.rs`'s `Kind::Process` scenarios.
//! Here: reading back the coordinator's own last log line from a real file.

use super::last_log_line;
use crate::config::Paths;

#[test]
fn the_last_non_empty_log_line_is_read_back_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    assert_eq!(last_log_line(&paths), None, "no log yet");
    std::fs::create_dir_all(dir.path().join("logs")).unwrap();
    std::fs::write(
        dir.path().join("logs/sync-daemon.err.log"),
        "first\n  last line with spaces  \n\n   \n",
    )
    .unwrap();
    assert_eq!(
        last_log_line(&paths).as_deref(),
        Some("last line with spaces")
    );
}
