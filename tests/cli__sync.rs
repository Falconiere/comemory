#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Integration tests for `comemory sync` (cloud push/pull CLI).

#[path = "common/auth_fixture.rs"]
mod auth_fixture;
#[path = "common/sync_platform_server.rs"]
mod sync_platform_server;

use assert_cmd::Command;
use predicates::prelude::*;
use sync_platform_server::{SyncPlatformServer, SyncPlatformState};
use tempfile::TempDir;

fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

#[test]
fn sync_without_login_is_usage_error() {
    let home = TempDir::new().unwrap();
    bin(&home)
        .args(["sync", "--action", "status"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not logged in"));
}

#[test]
fn sync_action_help_lists_push_and_status() {
    let home = TempDir::new().unwrap();
    bin(&home)
        .args(["sync", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--action"))
        .stdout(predicate::str::contains("--allow-secret"))
        // The org-scoped key names its own workspace, so there is nothing to
        // point `--workspace` at any more.
        .stdout(predicate::str::contains("--workspace").not());
}

#[test]
fn sync_rejects_a_credential_written_before_org_scoping() {
    // A v1 auth.json holds an unbound device key the platform no longer
    // accepts. `sync` is a command the user ran on purpose, so it says so
    // rather than silently doing nothing.
    let home = TempDir::new().unwrap();
    let data_dir = home.path().join(".comemory");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::write(
        data_dir.join("auth.json"),
        r#"{"secret":"cmk_legacy","key_prefix":"cmk_lega","personal_workspace_id":"ws-personal","api_url":"https://api.comemory.io","device_name":"laptop"}"#,
    )
    .unwrap();

    let out = bin(&home)
        .args(["sync", "--action", "status"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(out.status.code(), Some(64), "Error::Usage maps to EX_USAGE");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("comemory auth login"), "stderr={err}");
    assert!(err.contains("organization scoping"), "stderr={err}");
}

#[test]
fn sync_rejects_an_unknown_workspace_argument() {
    // The org-scoped key names its own workspace, so there is nothing to point
    // `--workspace` at. clap reports it; the exit code is clap's own, not one
    // `src/main.rs` maps, so assert on the message.
    let home = TempDir::new().unwrap();
    let out = bin(&home)
        .args(["sync", "--action", "push", "--workspace", "ws-somewhere"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("--workspace"),
        "clap must name the unexpected argument: {err}"
    );
}

#[test]
fn sync_loads_a_config_carrying_every_deprecated_key() {
    // `PartialSyncConfig` is deny_unknown_fields. If the removed keys stopped
    // parsing, upgrading would brick every installation that ran the old
    // `comemory link`.
    let home = TempDir::new().unwrap();
    let data_dir = home.path().join(".comemory");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::write(
        data_dir.join("config.toml"),
        concat!(
            "[sync]\n",
            "allowlist_ttl = \"2h\"\n",
            "default_workspace = \"ws_old\"\n",
            "\n",
            "[sync.repos]\n",
            "\"acme/backend\" = \"ws_old\"\n",
        ),
    )
    .unwrap();

    // Still reaches the login check rather than failing to load the config.
    bin(&home)
        .args(["sync", "--action", "status"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not logged in"));
}

/// A data dir holding an org credential for `server`, as `auth login` leaves it.
fn logged_in(server: &SyncPlatformServer) -> TempDir {
    let home = TempDir::new().unwrap();
    let paths = comemory::config::Paths::new(home.path().join(".comemory"));
    paths.ensure_dirs().unwrap();
    auth_fixture::seed_org_auth(
        &paths,
        &server.base,
        &server.snapshot().secret,
        auth_fixture::FIXTURE_WORKSPACE,
    );
    home
}

fn sync_json(home: &TempDir, args: &[&str]) -> serde_json::Value {
    let out = bin(home)
        .arg("sync")
        .args(args)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "sync {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

/// The default action through the built binary: every leg runs, including the
/// hooked-repo refresh that precedes the code push, and the platform sees the
/// pull before anything is pushed.
#[test]
fn a_default_run_reports_every_leg_and_pulls_first() {
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let home = logged_in(&server);

    let report = sync_json(&home, &[]);

    assert_eq!(report["workspace"], auth_fixture::FIXTURE_WORKSPACE);
    for leg in ["pull", "push", "code", "refresh"] {
        assert!(report[leg].is_object(), "{leg} ran: {report}");
    }
    assert!(server.saw_path("/v1/sync/changes"), "{:?}", server.paths());
}

#[test]
fn a_pull_action_pulls_and_pushes_nothing() {
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let home = logged_in(&server);

    let report = sync_json(&home, &["--action", "pull"]);

    assert!(report["pull"].is_object(), "{report}");
    assert!(
        report["push"].is_null() && report["code"].is_null(),
        "{report}"
    );
    assert!(server.saw_path("/v1/sync/changes"));
    assert!(!server.saw_path("/v1/sync/import"), "{:?}", server.paths());
}

#[test]
fn a_verify_action_compares_manifests() {
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let home = logged_in(&server);

    let report = sync_json(&home, &["--action", "verify"]);

    assert_eq!(report["differing_buckets"], 0, "{report}");
    assert!(server.saw_path("/v1/sync/manifest"), "{:?}", server.paths());
}
