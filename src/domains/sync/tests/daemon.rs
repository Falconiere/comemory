#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Daemon unit rendering + status detail (no live launchd/systemd required).

use std::path::Path;

use comemory::domains::sync::daemon::readiness::Trigger;
use comemory::domains::sync::daemon::{
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

/// One cycle is the same pass `sync --action auto` runs: a hooked repo whose
/// HEAD moved with no hook firing is re-indexed and its code pushed, with no
/// process ever started inside the checkout.
#[test]
fn a_cycle_refreshes_a_stale_hooked_repo_and_pushes_its_code() {
    use crate::test_common as common;
    use crate::test_common::code_sync_fixture as fixture;
    use crate::test_common::sync_platform_server::{SyncPlatformServer, SyncPlatformState};

    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let home = tempfile::tempdir().unwrap();
    let paths = comemory::config::Paths::new(home.path().join("data"));
    paths.ensure_dirs().unwrap();
    let cfg = comemory::config::Config::defaults();
    common::auth_fixture::seed_org_auth(
        &paths,
        &server.base,
        &server.snapshot().secret,
        common::auth_fixture::FIXTURE_WORKSPACE,
    );
    let tree = fixture::write_ts_repo(home.path());
    fixture::install_inert_hooks(&paths, &cfg, &tree);
    let mut conn = comemory::store::connection::open(paths.db_path()).unwrap();
    fixture::index(&paths, &cfg, &mut conn, &tree);
    drop(conn);
    let head = fixture::commit_hookless(
        &tree,
        &[(
            "src/c.ts",
            "export function gamma(): number {\n  return 3;\n}\n",
        )],
        "touch c",
    );

    let summary = super::worker::pass(&paths, Trigger::Tick, &[]);
    assert_eq!(summary.error, None, "{summary:?}");
    assert!(summary.logged_in);

    let conn = comemory::store::connection::open(paths.db_path()).unwrap();
    assert_eq!(
        comemory::store::repo_marker::last_head(&conn, fixture::REPO)
            .unwrap()
            .as_deref(),
        Some(head.as_str())
    );
    let bodies = server.snapshot().code_import_bodies;
    assert_eq!(bodies.len(), 1, "the refreshed repo is pushed once");
    assert!(bodies[0].contains(&head), "pushed at the new head");
}

/// Logged out, a cycle still re-indexes a stale hooked repo — the local half
/// never needed a credential — and makes no network call: there is no
/// platform to reach, and the cycle still succeeds.
#[test]
fn a_logged_out_cycle_refreshes_locally_and_succeeds() {
    use crate::test_common::code_sync_fixture as fixture;

    let home = tempfile::tempdir().unwrap();
    let paths = comemory::config::Paths::new(home.path().join("data"));
    paths.ensure_dirs().unwrap();
    let cfg = comemory::config::Config::defaults();
    let tree = fixture::write_ts_repo(home.path());
    fixture::install_inert_hooks(&paths, &cfg, &tree);
    let mut conn = comemory::store::connection::open(paths.db_path()).unwrap();
    fixture::index(&paths, &cfg, &mut conn, &tree);
    drop(conn);
    let head = fixture::commit_hookless(
        &tree,
        &[(
            "src/a.ts",
            "export function alpha(): number {\n  return 9;\n}\n",
        )],
        "touch a",
    );
    assert!(!paths.auth_file().exists(), "this machine is logged out");

    let summary = super::worker::pass(&paths, Trigger::Tick, &[]);
    assert_eq!(summary.error, None, "a logged-out pass is not an error");
    assert!(!summary.logged_in);

    let conn = comemory::store::connection::open(paths.db_path()).unwrap();
    assert_eq!(
        comemory::store::repo_marker::last_head(&conn, fixture::REPO)
            .unwrap()
            .as_deref(),
        Some(head.as_str())
    );
}
