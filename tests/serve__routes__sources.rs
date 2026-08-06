//! End-to-end coverage of `GET /api/v1/sources` (`src/serve/routes/sources.rs`)
//! against a real bound server, including the read-only reconcile gate: the
//! route computes `reconcile` from `--read-only` server-side, never from the
//! query string (mirrors `tests/api__sources.rs`, which exercises
//! `api::sources::run` directly without HTTP).

#[path = "common/docs_fixtures.rs"]
mod docs_fixtures;

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
/// session token, and the kill-on-drop guard. `extra_args` is appended after
/// `serve --port 0` (e.g. `&["--read-only"]`).
fn spawn_serve(home: &TempDir, extra_args: &[&str]) -> (String, String, ServerGuard) {
    let mut child = Command::new(cargo_bin("comemory"))
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args(["--json", "serve", "--port", "0"])
        .args(extra_args)
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

/// Register one real docs fixture directory as a source in `home`; returns
/// the fixture workspace kept alive for the duration of the test.
fn seeded_home() -> (TempDir, TempDir) {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());
    AssertCommand::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args([
            "index",
            docs.to_str().expect("utf8 path"),
            "--repo",
            "docs-corpus",
        ])
        .assert()
        .success();
    (home, workspace)
}

/// Drop every entry from `sources.toml`, simulating an out-of-band edit that
/// leaves the mirror row registered but the durable registry empty.
fn clear_registry(home: &TempDir) {
    std::fs::write(
        home.path().join(".comemory").join("sources.toml"),
        "format = 1\n",
    )
    .expect("rewrite sources.toml");
}

#[test]
fn v1_sources_returns_the_registered_source_envelope() {
    let (home, _workspace) = seeded_home();
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/sources"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 sources");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["meta"]["command"], "sources");

    let rows = body["data"].as_array().expect("data array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["repo"], "docs-corpus");
    assert_eq!(
        rows[0]["indexed"].as_u64(),
        Some(docs_fixtures::FIXTURE_COUNT as u64)
    );
}

#[test]
fn v1_sources_read_only_server_skips_the_mirror_reconcile() {
    let (home, _workspace) = seeded_home();
    clear_registry(&home);
    let (base, token, _guard) = spawn_serve(&home, &["--read-only"]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/sources"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 sources read-only");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    let rows = body["data"].as_array().expect("data array");
    assert_eq!(
        rows.len(),
        1,
        "a read-only server must not reconcile away the now-unregistered row"
    );
}
