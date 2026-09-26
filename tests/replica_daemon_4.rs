#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The required resident daemon (#257), part 4: lifecycle and introspection
//! commands run no preflight at all (A-7), a required-preflight command
//! against an empty data directory never creates the corpus (D9/A-7), and
//! `auth login`/`auth logout` (S8) drive the same coordinator through
//! `reload` rather than the pre-#257 opt-in install.

#[path = "common/auth_fixture.rs"]
mod auth_fixture;
#[path = "common/daemon_support.rs"]
mod daemon_support;
#[path = "common/device_auth_server.rs"]
mod device_auth_server;
#[path = "common/release_server.rs"]
mod release_server;

use std::time::Duration;

use comemory::domains::sync::daemon::client;
use comemory::domains::sync::daemon::control::Op;
use comemory::domains::sync::daemon::readiness::AuthState;
use daemon_support::DaemonHome;
use device_auth_server::DeviceAuthServer;
use release_server::ReleaseServer;
use serde_json::Value;

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

/// AC-4: writing a credential and sending `reload` — what `auth login`
/// does internally — flips the coordinator's own `auth.state`, and a second
/// workspace written the same way replaces the first rather than merging
/// with it.
#[test]
fn auth_lifecycle_reload_flips_auth_state_and_the_next_workspace_replaces_the_last() {
    let home = DaemonHome::new();
    home.json(&["sync", "daemon", "ensure"]);
    let started = home.wait_ready(Duration::from_secs(15));
    assert_eq!(started.auth.state, AuthState::LoggedOut, "{started:?}");

    auth_fixture::seed_org_auth(
        &home.paths(),
        "http://127.0.0.1:9/api",
        "cmk_first",
        "ws_first",
    );
    client::call::<Value>(&home.paths(), &Op::Reload {}, Duration::from_secs(2)).unwrap();
    let first = home.wait_for(Duration::from_secs(5), |r| {
        r.auth.state == AuthState::Authenticated
    });
    assert_eq!(first.auth.workspace_id.as_deref(), Some("ws_first"));
    assert_eq!(
        first.instance, started.instance,
        "reload updates the running coordinator; it never starts a new one"
    );

    auth_fixture::seed_org_auth(
        &home.paths(),
        "http://127.0.0.1:9/api",
        "cmk_second",
        "ws_second",
    );
    client::call::<Value>(&home.paths(), &Op::Reload {}, Duration::from_secs(2)).unwrap();
    // `AuthView` names exactly one workspace, so seeing it flip from
    // `ws_first` to `ws_second` (not, say, still `ws_first`, and not an
    // error) is what "replaces, never merges" looks like from the outside.
    let second = home.wait_for(Duration::from_secs(5), |r| {
        r.auth.workspace_id.as_deref() == Some("ws_second")
    });
    assert_ne!(
        second.auth.workspace_id, first.auth.workspace_id,
        "{second:?}"
    );
}

/// AC-4/D11: the logout barrier flips the coordinator to
/// `logged_out_barrier`, blocks a local `auth status` even with an
/// inherited `COMEMORY_API_KEY`, and never stops the coordinator itself.
#[test]
fn logout_barrier_blocks_every_credential_read_and_leaves_the_coordinator_running() {
    let home = DaemonHome::new();
    home.json(&["sync", "daemon", "ensure"]);
    let started = home.wait_ready(Duration::from_secs(15));
    auth_fixture::seed_org_auth(&home.paths(), "http://127.0.0.1:9/api", "cmk_x", "ws_x");

    let logout = home.json(&["auth", "logout"]);
    assert_eq!(logout["logged_out"], true, "{logout}");
    assert_eq!(
        logout["daemon"]["running"], true,
        "logout never stops the coordinator (D11): {logout}"
    );
    assert!(home.data_dir().join("auth.disabled").exists());
    assert!(!home.data_dir().join("auth.json").exists());

    // No credential read succeeds while the barrier stands, even one an
    // inherited COMEMORY_API_KEY would otherwise satisfy.
    let (code, stdout, _) = home.run_with_env(
        &["--json", "auth", "status"],
        &[("COMEMORY_API_KEY", "cmk_env_leak")],
    );
    assert_ne!(code, 0);
    let body: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(body["authenticated"], false, "{body}");

    let after = home.wait_for(Duration::from_secs(5), |r| {
        r.auth.state == AuthState::LoggedOutBarrier
    });
    assert_eq!(
        after.instance, started.instance,
        "the same instance is running afterwards (D11)"
    );
}

/// AC-8/compat: `--daemon` is a parseable, deprecated no-op — the
/// coordinator preflight already ensured is what answers, not an install
/// this flag triggers — and a real device login still reaches it through
/// `reload` with no second command.
#[test]
fn compat_the_deprecated_daemon_flag_changes_nothing_and_login_still_reaches_the_coordinator() {
    if !device_auth_server::tooling_present() {
        return;
    }
    let home = DaemonHome::new();
    let srv = DeviceAuthServer::start_default();

    let (code, stdout, stderr) = home.run(&[
        "--json",
        "auth",
        "login",
        "--daemon",
        "--api-url",
        &srv.base,
    ]);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stderr.contains("deprecated"),
        "--daemon must warn it is a no-op: stderr={stderr}"
    );
    let report: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["daemon"]["skipped"], false, "{report}");
    assert_eq!(
        report["daemon"]["running"], true,
        "preflight already ensured a coordinator for this Required command: {report}"
    );
    assert!(report["daemon"]["instance"].as_str().is_some(), "{report}");

    let status = home.json(&["sync", "daemon", "status"]);
    assert_eq!(
        status["daemon"]["auth"]["state"], "authenticated",
        "the coordinator picked up the credential without a second command: {status}"
    );
}
