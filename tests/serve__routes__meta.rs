#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! End-to-end coverage of `GET /api/v1/completions` and `GET /api/v1/commands`
//! (`src/serve/routes/meta.rs`) against a real bound server (mirrors
//! `tests/api__completions.rs`, which exercises `api::completions::run`
//! directly without HTTP).

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

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
fn v1_completions_returns_the_generated_script_envelope() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/completions?shell=bash"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 completions");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["meta"]["command"], "completions");
    let script = body["data"].as_str().expect("data is a string");
    assert!(
        script.contains("comemory"),
        "completions script must mention the binary name: {script}"
    );
}

#[test]
fn v1_completions_rejects_an_unsupported_shell() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/completions?shell=not-a-shell"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 completions bad shell");
    assert!(
        !res.status().is_success(),
        "unsupported shell must not succeed: {}",
        res.status()
    );
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(false));
    assert_eq!(body["error"]["code"], "bad_request");
}

#[test]
fn v1_commands_lists_every_subcommand_with_its_routes() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/commands"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 commands");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["meta"]["command"], "commands");

    let commands = body["data"]["commands"].as_array().expect("commands array");

    let serve_entry = commands
        .iter()
        .find(|c| c["name"] == "serve")
        .expect("serve entry present");
    assert_eq!(serve_entry["transport"], "cli-only");
    assert_eq!(serve_entry["routes"], serde_json::json!([]));

    let doctor_entry = commands
        .iter()
        .find(|c| c["name"] == "doctor")
        .expect("doctor entry present");
    assert_eq!(doctor_entry["transport"], "http");
    let doctor_routes = doctor_entry["routes"].as_array().expect("routes array");
    assert!(
        !doctor_routes.is_empty(),
        "doctor must have at least one wired route"
    );
    assert!(
        doctor_routes
            .iter()
            .any(|r| r.as_str() == Some("GET /api/v1/doctor")),
        "expected GET /api/v1/doctor in {doctor_routes:?}"
    );
}
