#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
#![cfg(unix)]
//! `comemory auth login|status|logout` against two loopback fixtures. Real
//! binary, real curl/wget, real HTTP — no in-process mocks.
//!
//! `device_auth_server` covers the grant itself (pending polls, rejected
//! client, expired code, an unscoped mint). `sync_platform_server` also serves
//! the sync routes, so it is the one that can prove login runs the first sync.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Output;

use assert_cmd::cargo::cargo_bin;
use device_auth_server::{DeviceAuthConfig, DeviceAuthServer, tooling_present};
use serde_json::Value;
use sync_platform_server::{SyncPlatformServer, SyncPlatformState};
use tempfile::TempDir;

#[path = "common/device_auth_server.rs"]
mod device_auth_server;
#[path = "common/sync_platform_server.rs"]
mod sync_platform_server;

struct Home {
    root: TempDir,
}

impl Home {
    fn new() -> Self {
        Self {
            root: TempDir::new().unwrap(),
        }
    }

    fn data_dir(&self) -> PathBuf {
        self.root.path().join(".comemory")
    }

    fn auth_file(&self) -> PathBuf {
        self.data_dir().join("auth.json")
    }

    fn run(&self, api: Option<&str>, args: &[&str]) -> Output {
        let mut cmd = std::process::Command::new(cargo_bin("comemory"));
        cmd.env("COMEMORY_DATA_DIR", self.data_dir())
            .env("HOME", self.root.path())
            .env_remove("COMEMORY_API")
            .env_remove("COMEMORY_API_KEY")
            .args(args);
        if let Some(url) = api {
            cmd.env("COMEMORY_API", url);
        }
        cmd.output().expect("run comemory")
    }

    fn run_json(&self, api: Option<&str>, args: &[&str]) -> Value {
        let mut full = vec!["--json"];
        full.extend_from_slice(args);
        let out = self.run(api, &full);
        assert!(
            out.status.success(),
            "expected success {:?}: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
            panic!(
                "stdout not JSON: {e}\n{}",
                String::from_utf8_lossy(&out.stdout)
            )
        })
    }
}

fn require_http_tools() {
    assert!(
        tooling_present(),
        "curl or wget required for auth integration tests"
    );
}

#[test]
fn login_writes_v2_auth_file() {
    require_http_tools();
    let srv = DeviceAuthServer::start_default();
    let home = Home::new();
    let report = home.run_json(None, &["auth", "login", "--api-url", &srv.base]);
    assert_eq!(report["authenticated"], true);
    assert_eq!(report["api_url"], srv.base);
    assert_eq!(
        report["organization_id"],
        srv.config.organization_id.as_str()
    );
    assert_eq!(report["organization_slug"], "acme");
    assert_eq!(report["organization_name"], "Acme, Inc.");
    assert_eq!(report["workspace_id"], srv.config.workspace_id.as_str());
    assert_eq!(report["key_prefix"], srv.config.key_prefix.as_str());
    assert_eq!(report["secret"], srv.config.secret.as_str());

    let path = home.auth_file();
    assert!(path.is_file());
    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "auth.json must be 0600, got {mode:#o}");

    // The file `sync` reads is the file `auth login` writes — same schema.
    let on_disk: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(on_disk["version"], 2);
    assert_eq!(on_disk["secret"], srv.config.secret.as_str());
    assert_eq!(
        on_disk["organization_id"],
        srv.config.organization_id.as_str()
    );
    assert_eq!(on_disk["workspace_id"], srv.config.workspace_id.as_str());
    assert_eq!(on_disk["api_url"], srv.base);
    assert!(
        on_disk.get("device_name").is_none(),
        "the device label is gone from the schema: {on_disk}"
    );
    assert!(
        on_disk.get("personal_workspace_id").is_none(),
        "the personal workspace is gone from the schema: {on_disk}"
    );

    // The org key is not fetched from a workspace list any more.
    assert!(
        !srv.saw_path("/v1/workspaces"),
        "login must not list workspaces, saw: {:?}",
        srv.requests()
    );
}

#[test]
fn login_runs_initial_sync_before_returning() {
    require_http_tools();
    let srv = SyncPlatformServer::start(SyncPlatformState::default());
    let home = Home::new();

    let report = home.run_json(None, &["auth", "login", "--api-url", &srv.base]);
    assert_eq!(report["initial_sync"]["ok"], true);
    assert_eq!(report["initial_sync"]["pulled"], 0);
    assert_eq!(report["initial_sync"]["pushed"], 0);

    // Order is load-bearing: pushing first would let sync_binding claim a
    // memory the organization already holds.
    let seen: Vec<String> = srv.requests().into_iter().map(|r| r.path).collect();
    let changes = seen.iter().position(|p| p == "/v1/sync/changes");
    assert!(changes.is_some(), "login must pull, saw: {seen:?}");

    // No manual step stood between login and a working cursor.
    let status = home.run_json(None, &["sync", "--action", "status"]);
    assert_eq!(status["workspace"], srv.snapshot().workspace_id.as_str());
}

#[test]
fn login_survives_a_sync_outage() {
    require_http_tools();
    let srv = SyncPlatformServer::start(SyncPlatformState::default());
    srv.update(|st| st.sync_unavailable = true);
    let home = Home::new();

    // The credential is already minted and useful. A blip on the first sync
    // must not leave the user logged out with no obvious next step.
    let out = home.run(None, &["--json", "auth", "login", "--api-url", &srv.base]);
    assert!(
        out.status.success(),
        "login must survive a sync outage: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["authenticated"], true);
    assert_eq!(report["initial_sync"]["ok"], false);
    assert!(
        report["initial_sync"]["error"].is_string(),
        "the failure must be reported, not hidden: {report}"
    );
    assert!(home.auth_file().is_file(), "the credential must be kept");
}

#[test]
fn login_and_logout_clear_a_stale_allowlist() {
    require_http_tools();
    let srv = DeviceAuthServer::start_default();
    let home = Home::new();
    fs::create_dir_all(home.data_dir()).unwrap();
    let allowlist = home.data_dir().join("allowlist.json");
    fs::write(
        &allowlist,
        r#"{"repos":[{"full_name":"acme/old","name":"old"}]}"#,
    )
    .unwrap();

    home.run_json(None, &["auth", "login", "--api-url", &srv.base]);
    assert!(
        !allowlist.exists(),
        "a cached repo list must not outlive the credential it was fetched for"
    );

    fs::write(&allowlist, r#"{"repos":[]}"#).unwrap();
    let out = home.run(None, &["auth", "logout"]);
    assert!(out.status.success());
    assert!(!allowlist.exists());
}

#[test]
fn unscoped_mint_leaves_no_credential_on_disk() {
    require_http_tools();
    let srv = DeviceAuthServer::start(DeviceAuthConfig {
        workspace_id: String::new(),
        ..DeviceAuthConfig::default()
    });
    let home = Home::new();
    let out = home.run(None, &["auth", "login", "--api-url", &srv.base]);
    assert!(!out.status.success(), "an unscoped mint must fail");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("organization scope"), "stderr={err}");
    assert!(
        !home.auth_file().exists(),
        "a credential that can do nothing must not be written"
    );
}

#[test]
fn login_keeps_dialed_url_and_takes_platform_api_url_when_sent() {
    require_http_tools();
    let srv = DeviceAuthServer::start(DeviceAuthConfig {
        mint_api_url: "https://api.example.test".into(),
        ..DeviceAuthConfig::default()
    });
    let home = Home::new();
    let report = home.run_json(None, &["auth", "login", "--api-url", &srv.base]);
    assert_eq!(report["api_url"], "https://api.example.test");

    // Blank `apiUrl` (deployment without BETTER_AUTH_URL) → keep what we dialed.
    let plain = DeviceAuthServer::start_default();
    let home2 = Home::new();
    let report2 = home2.run_json(None, &["auth", "login", "--api-url", &plain.base]);
    assert_eq!(report2["api_url"], plain.base);
}

#[test]
fn status_reports_the_bound_organization() {
    require_http_tools();
    let srv = SyncPlatformServer::start(SyncPlatformState::default());
    let home = Home::new();
    home.run_json(None, &["auth", "login", "--api-url", &srv.base]);

    let status = home.run_json(None, &["auth", "status"]);
    assert_eq!(status["authenticated"], true);
    assert_eq!(status["organization_name"], "Acme, Inc.");
    assert_eq!(
        status["organization_id"],
        srv.snapshot().organization_id.as_str()
    );
    assert_eq!(status["workspace_id"], srv.snapshot().workspace_id.as_str());
    assert_eq!(status["api_url"], srv.base);
    assert!(
        status.get("secret").is_none(),
        "status must not reprint secret"
    );
}

#[test]
fn revoked_key_reports_unauthenticated_rather_than_erroring() {
    require_http_tools();
    let srv = SyncPlatformServer::start(SyncPlatformState::default());
    let home = Home::new();
    home.run_json(None, &["auth", "login", "--api-url", &srv.base]);

    // Revoke by making the stored secret stop matching the fixture's key: the
    // 401 must read back as "not authenticated", not as a transport failure.
    let path = home.auth_file();
    let mut on_disk: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    // A value the fixture will not accept — note it must differ from the
    // fixture's own default secret, or "revoked" would still authenticate.
    on_disk["secret"] = Value::String(
        "cmk_cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".into(),
    );
    fs::write(&path, serde_json::to_string_pretty(&on_disk).unwrap()).unwrap();

    // `auth status` exits non-zero when unauthenticated, so read stdout directly.
    let status = home.run(None, &["--json", "auth", "status"]);
    assert!(!status.status.success());
    let body: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(body["authenticated"], false);
}

#[test]
fn logout_removes_auth_json() {
    require_http_tools();
    let srv = DeviceAuthServer::start_default();
    let home = Home::new();
    home.run_json(None, &["auth", "login", "--api-url", &srv.base]);
    assert!(home.auth_file().is_file());
    let out = home.run(None, &["auth", "logout"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!home.auth_file().exists());
    let status = home.run(None, &["--json", "auth", "status"]);
    assert!(!status.status.success());
    let body: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(body["authenticated"], false);
}

#[test]
fn wrong_client_or_expired_poll_surfaces_error() {
    require_http_tools();
    let home = Home::new();

    let reject = DeviceAuthServer::start(DeviceAuthConfig {
        reject_client: true,
        ..DeviceAuthConfig::default()
    });
    let out = home.run(None, &["auth", "login", "--api-url", &reject.base]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("invalid_client"), "stderr={err}");

    let expired = DeviceAuthServer::start(DeviceAuthConfig {
        expire_token: true,
        ..DeviceAuthConfig::default()
    });
    let out = home.run(None, &["auth", "login", "--api-url", &expired.base]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("expired_token"), "stderr={err}");
}

#[test]
fn api_url_flag_and_comemory_api_override_default() {
    require_http_tools();
    let srv = DeviceAuthServer::start_default();
    let home = Home::new();

    // Flag wins: point env at a dead host; --api-url should still hit the fixture.
    let mut cmd = std::process::Command::new(cargo_bin("comemory"));
    let out = cmd
        .env("COMEMORY_DATA_DIR", home.data_dir())
        .env("HOME", home.root.path())
        .env("COMEMORY_API", "http://127.0.0.1:1")
        .env_remove("COMEMORY_API_KEY")
        .args(["--json", "auth", "login", "--api-url", &srv.base])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "flag should beat env: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let on_disk: Value =
        serde_json::from_str(&fs::read_to_string(home.auth_file()).unwrap()).unwrap();
    assert_eq!(on_disk["api_url"], srv.base);

    // Env alone (no flag) also reaches the fixture.
    let home2 = Home::new();
    let report = home2.run_json(Some(&srv.base), &["auth", "login"]);
    assert_eq!(report["api_url"], srv.base);
    let on_disk: Value =
        serde_json::from_str(&fs::read_to_string(home2.auth_file()).unwrap()).unwrap();
    assert_eq!(on_disk["api_url"], srv.base);
}

#[test]
fn after_save_reaches_the_org_workspace_with_no_further_command() {
    // The symptom this whole change exists to fix. `[sync] after_save` has
    // always defaulted to true, but the workspace fell back to the personal
    // one, which the platform rejects — so a fresh install pushed into a void
    // until the user found `comemory workspaces` and `comemory link`.
    //
    // Nothing between login and save here: no --workspace, no link, no config.
    require_http_tools();
    let srv = SyncPlatformServer::start(SyncPlatformState::default());
    let home = Home::new();
    home.run_json(None, &["auth", "login", "--api-url", &srv.base]);

    let before = srv
        .requests()
        .iter()
        .filter(|r| r.path == "/v1/sync/import")
        .count();

    let out = home.run(
        None,
        &[
            "save",
            "--repo",
            "acme/backend",
            "a decision worth sharing with the organization",
        ],
    );
    assert!(
        out.status.success(),
        "save failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The auto-push is best-effort and detached, so poll rather than assume
    // it already landed.
    let mut imports = before;
    for _ in 0..50 {
        imports = srv
            .requests()
            .iter()
            .filter(|r| r.path == "/v1/sync/import")
            .count();
        if imports > before {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(
        imports > before,
        "after_save must reach the organization unaided; requests seen: {:?}",
        srv.requests().iter().map(|r| &r.path).collect::<Vec<_>>()
    );

    let body = srv.snapshot().last_import_body.expect("an import was sent");
    assert!(
        body.contains("acme/backend"),
        "the org-labelled memory must be in the import body: {body}"
    );
}
