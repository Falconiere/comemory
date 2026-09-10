#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! SessionEnd hook installer.

use comemory::capture::hook::{HOOK_MARKER, install};

#[test]
fn install_writes_session_end_command() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    let report = install(&path, false).expect("install");
    assert!(report.command.contains(HOOK_MARKER));
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.contains("SessionEnd"));
    assert!(raw.contains(HOOK_MARKER));
    // Idempotent refresh
    install(&path, false).expect("reinstall");
    let raw2 = std::fs::read_to_string(&path).unwrap();
    assert_eq!(raw2.matches(HOOK_MARKER).count(), 1);
}
