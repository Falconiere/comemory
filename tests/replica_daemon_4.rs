#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The required resident daemon (#257), part 4: lifecycle and introspection
//! commands run no preflight at all (A-7), and a required-preflight command
//! against an empty data directory never creates the corpus (D9/A-7).

#[path = "common/daemon_support.rs"]
mod daemon_support;
#[path = "common/release_server.rs"]
mod release_server;

use std::time::Duration;

use daemon_support::DaemonHome;
use release_server::ReleaseServer;

/// No `daemon.lock`, `daemon.sock` or `daemon.json` exists for `home`.
fn no_coordinator_artifacts(home: &DaemonHome) {
    for name in ["daemon.lock", "daemon.sock", "daemon.json"] {
        assert!(
            !home.data_dir().join(name).exists(),
            "{name} must not exist"
        );
    }
    assert!(home.coordinator_pids().is_empty());
}

#[test]
fn every_exempt_command_starts_no_coordinator() {
    let home = DaemonHome::new();
    let (code, _, stderr) = home.run(&["completions", "zsh"]);
    assert_eq!(code, 0, "{stderr}");
    no_coordinator_artifacts(&home);
    home.json(&["doctor"]);
    no_coordinator_artifacts(&home);
    // Logged out, `auth status` exits nonzero (not authenticated) — the
    // point here is only that it never touches the daemon.
    home.run(&["auth", "status"]);
    no_coordinator_artifacts(&home);
    // Logged out, `--action status` also needs a credential to open its
    // session — again, only the daemon-touching question matters here.
    home.run(&["sync", "--action", "status"]);
    no_coordinator_artifacts(&home);
    home.json(&["sync", "daemon", "status"]);
    no_coordinator_artifacts(&home);

    // Only whether `upgrade --check` touches the daemon matters here, not
    // whether it finds an upgrade — no release is staged under this tag.
    let root = home.root().join("releases");
    std::fs::create_dir_all(&root).unwrap();
    let server = ReleaseServer::start(root, "0.0.0-nonexistent");
    home.run_with_env(
        &["--json", "upgrade", "--check"],
        &[("COMEMORY_RELEASES_URL", &server.base)],
    );
    no_coordinator_artifacts(&home);

    let mut serving = home.spawn_serve();
    std::thread::sleep(Duration::from_millis(300));
    no_coordinator_artifacts(&home);
    let _ = serving.kill();
    let _ = serving.wait();
}

#[test]
fn a_required_command_over_an_empty_directory_never_creates_the_corpus() {
    let home = DaemonHome::new();
    let mut mcp = home.spawn_mcp_read_only();
    let ready = home.wait_ready(Duration::from_secs(15));
    assert_eq!(
        ready.store,
        comemory::domains::sync::daemon::readiness::StoreState::Absent,
        "the coordinator never opens the corpus for an empty directory"
    );
    let _ = mcp.kill();
    let _ = mcp.wait();
}
