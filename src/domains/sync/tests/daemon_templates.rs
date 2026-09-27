#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Rendering-only tests for the per-directory unit templates (#257): no
//! `launchctl`/`systemctl` involved, just the text each backend writes.

use std::path::Path;

use super::{render_launch_agent_plist, render_systemd_unit};

#[test]
fn launch_agent_plist_names_label_data_dir_and_run_args() {
    let body = render_launch_agent_plist(
        "io.comemory.sync.abc123",
        Path::new("/usr/local/bin/comemory"),
        Path::new("/tmp/cm"),
    );
    assert!(body.contains("io.comemory.sync.abc123"));
    assert!(body.contains("/usr/local/bin/comemory"));
    assert!(body.contains("<string>sync</string>"));
    assert!(body.contains("<string>daemon</string>"));
    assert!(body.contains("<string>run</string>"));
    assert!(body.contains("COMEMORY_DATA_DIR"));
    assert!(body.contains("/tmp/cm"));
    assert!(body.contains("KeepAlive"));
    assert!(body.contains("<false/>"), "SuccessfulExit is false (D10)");
    assert!(
        body.contains("<key>COMEMORY_DAEMON_SUPERVISOR</key>\n    <string>launchd</string>"),
        "readiness reports the real backend (#258 D12)"
    );
    assert_eq!(
        body.matches("<key>COMEMORY_").count(),
        2,
        "no other env key"
    );
}

#[test]
fn systemd_unit_names_exec_and_data_dir() {
    let body = render_systemd_unit(
        Path::new("/usr/bin/comemory"),
        Path::new("/home/u/.comemory"),
    );
    assert!(
        body.contains("ExecStart=/usr/bin/comemory --data-dir /home/u/.comemory sync daemon run")
    );
    assert!(body.contains("COMEMORY_DATA_DIR=/home/u/.comemory"));
    assert!(body.contains("WantedBy=default.target"));
    assert!(body.contains("Restart=on-failure"));
    assert!(body.contains("Environment=COMEMORY_DAEMON_SUPERVISOR=systemd"));
    assert_eq!(body.matches("Environment=").count(), 2, "no other env key");
}
