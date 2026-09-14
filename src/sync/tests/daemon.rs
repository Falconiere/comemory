#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Daemon unit rendering + status detail (no live launchd/systemd required).

use std::path::Path;

use comemory::sync::daemon::{
    DAEMON_LABEL, SYSTEMD_UNIT, render_launch_agent_plist, render_systemd_unit, status,
};

#[test]
fn launch_agent_plist_names_label_and_run_args() {
    let body =
        render_launch_agent_plist(Path::new("/usr/local/bin/comemory"), Path::new("/tmp/cm"));
    assert!(body.contains(DAEMON_LABEL));
    assert!(body.contains("/usr/local/bin/comemory"));
    assert!(body.contains("<string>sync</string>"));
    assert!(body.contains("<string>daemon</string>"));
    assert!(body.contains("<string>run</string>"));
    assert!(body.contains("COMEMORY_DATA_DIR"));
    assert!(body.contains("/tmp/cm"));
    assert!(body.contains("KeepAlive"));
}

#[test]
fn systemd_unit_names_service_and_exec() {
    let body = render_systemd_unit(
        Path::new("/usr/bin/comemory"),
        Path::new("/home/u/.comemory"),
    );
    assert!(body.contains("ExecStart=/usr/bin/comemory sync daemon run"));
    assert!(body.contains("COMEMORY_DATA_DIR=/home/u/.comemory"));
    assert!(body.contains("WantedBy=default.target"));
    assert_eq!(SYSTEMD_UNIT, "comemory-sync.service");
}

#[test]
fn status_reports_platform_without_panicking() {
    // May or may not be installed on the developer machine; just prove the
    // probe returns a structured report.
    let st = status().expect("status");
    assert!(
        matches!(st.platform, "macos" | "linux" | "unsupported"),
        "platform={}",
        st.platform
    );
    assert!(!st.detail.is_empty());
}
