#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
#![cfg(unix)]
//! `comemory auth login|status|logout` against a loopback device-auth
//! fixture (`tests/common/device_auth_server.rs`). Real binary, real
//! curl/wget, real HTTP — no in-process mocks.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Output;

use assert_cmd::cargo::cargo_bin;
use device_auth_server::{DeviceAuthConfig, DeviceAuthServer, tooling_present};
use serde_json::Value;
use tempfile::TempDir;

#[path = "common/device_auth_server.rs"]
mod device_auth_server;

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
fn login_writes_auth_json_mode_0600_and_usable_secret() {
    require_http_tools();
    let srv = DeviceAuthServer::start_default();
    let home = Home::new();
    let report = home.run_json(
        None,
        &[
            "auth",
            "login",
            "--api-url",
            &srv.base,
            "--device-name",
            "fixture-laptop",
        ],
    );
    assert_eq!(report["authenticated"], true);
    assert_eq!(report["api_url"], srv.base);
    assert_eq!(
        report["personal_workspace_id"],
        srv.config.workspace_id.as_str()
    );
    assert_eq!(report["device_name"], "fixture-laptop");
    assert_eq!(report["key_prefix"], srv.config.key_prefix.as_str());
    assert_eq!(report["secret"], srv.config.secret.as_str());

    let path = home.auth_file();
    assert!(path.is_file());
    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "auth.json must be 0600, got {mode:#o}");
    let on_disk: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(on_disk["secret"], srv.config.secret.as_str());
    // The file `sync` reads is the file `auth login` writes — same schema.
    assert_eq!(
        on_disk["personal_workspace_id"],
        srv.config.workspace_id.as_str()
    );
    assert_eq!(on_disk["device_name"], "fixture-laptop");
    assert_eq!(on_disk["api_url"], srv.base);
    let listed = home.run_json(None, &["workspaces"]);
    assert_eq!(
        listed["workspaces"][0]["id"],
        srv.config.workspace_id.as_str()
    );
    assert_eq!(listed["workspaces"][0]["personal"], true);
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
fn status_reports_authenticated_after_login() {
    require_http_tools();
    let srv = DeviceAuthServer::start_default();
    let home = Home::new();
    home.run_json(None, &["auth", "login", "--api-url", &srv.base]);
    let status = home.run_json(None, &["auth", "status"]);
    assert_eq!(status["authenticated"], true);
    assert_eq!(status["workspace_id"], srv.config.workspace_id.as_str());
    assert_eq!(status["key_prefix"], srv.config.key_prefix.as_str());
    assert_eq!(status["workspace_name"], srv.config.workspace_name.as_str());
    assert_eq!(status["api_url"], srv.base);
    assert!(
        status.get("secret").is_none(),
        "status must not reprint secret"
    );
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
