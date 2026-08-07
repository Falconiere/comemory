#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! End-to-end coverage of `GET /api/v1/doctor` and `GET /api/v1/consolidate`
//! (`src/serve/routes/maint/mod.rs`) against a real bound server.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use assert_cmd::Command as AssertCommand;
use assert_cmd::cargo::cargo_bin;
use tempfile::TempDir;

/// Kills the spawned server on drop so a panicking assertion cannot leak it.
struct ServerGuard(Child);
impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Spawn `comemory serve` on an ephemeral port, returning the base URL, the
/// session token, and the kill-on-drop guard.
fn spawn_serve(home: &TempDir) -> (String, String, ServerGuard) {
    let mut child = Command::new(cargo_bin("comemory"))
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args(["--json", "serve", "--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn serve");
    let stdout = child.stdout.take().expect("piped stdout");
    let mut line = String::new();
    BufReader::new(stdout)
        .read_line(&mut line)
        .expect("read banner");
    let guard = ServerGuard(child);
    let info: serde_json::Value = serde_json::from_str(line.trim()).expect("banner is json");
    let port = info["port"].as_u64().expect("port");
    let token = info["token"].as_str().expect("token").to_string();
    (format!("http://127.0.0.1:{port}"), token, guard)
}

#[test]
fn v1_doctor_returns_the_health_report_envelope() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/doctor"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 doctor");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["meta"]["command"], "doctor");
    assert_eq!(body["data"]["db_writable"], serde_json::json!(true));
    assert!(body["data"]["schema_version"].is_string(), "body: {body}");
    assert_eq!(body["data"]["sqlite_vec_loaded"], serde_json::json!(true));
}

/// Save `body` via the real binary, returning the assigned memory id.
fn save_id(home: &TempDir, body: &str) -> String {
    let out = AssertCommand::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args(["--json", "save", body, "--kind", "note", "--repo", "demo"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    serde_json::from_str::<serde_json::Value>(stdout.trim()).expect("save json")["id"]
        .as_str()
        .expect("id")
        .to_string()
}

#[test]
fn v1_consolidate_returns_the_cluster_envelope() {
    let home = TempDir::new().expect("home");
    for suffix in ["runner", "worker", "daemon"] {
        save_id(
            &home,
            &format!("postgres advisory lock ordering fix for the migration {suffix}"),
        );
    }
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/consolidate"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 consolidate");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["meta"]["command"], "consolidate");
    assert_eq!(body["data"]["scanned"].as_u64(), Some(3));
    assert_eq!(body["data"]["clusters"]["total"].as_u64(), Some(1));
}

#[test]
fn v1_consolidate_honors_the_radius_query_param() {
    let home = TempDir::new().expect("home");
    for suffix in ["runner", "worker"] {
        save_id(
            &home,
            &format!("postgres advisory lock ordering fix for the migration {suffix}"),
        );
    }
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/consolidate?radius=0"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 consolidate radius=0");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["data"]["radius"].as_u64(), Some(0));
    assert_eq!(body["data"]["clusters"]["total"].as_u64(), Some(0));
}
