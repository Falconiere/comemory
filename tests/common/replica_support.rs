#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    dead_code
)]
//! Real-process fixture for the `replica-v1` contract tests: a spawned
//! `comemory serve` per engine, its data directory, its CLI, and the helpers
//! that build operations from memories the CLI actually saved.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use assert_cmd::cargo::cargo_bin;
use serde_json::{Value, json};

/// Real README prose — representative content, not a fixture string.
pub const BODY: &str = "comemory keeps a durable, searchable memory of the \
decisions, bugs and conventions a codebase accumulates, and links them to the \
code they describe.";

/// Kills the spawned server on drop so a panicking assertion cannot leak it.
struct ServerGuard(Child);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// One engine: its data directory plus the `serve` process in front of it.
pub struct Engine {
    home: tempfile::TempDir,
    pub base: String,
    pub token: String,
    _guard: ServerGuard,
}

impl Engine {
    /// Spawn `comemory serve` on an ephemeral port over a fresh data dir.
    pub fn spawn(extra_args: &[&str]) -> Self {
        let home = tempfile::TempDir::new().expect("tempdir");
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
        let info: Value = serde_json::from_str(line.trim()).expect("banner is json");
        let port = info["port"].as_u64().expect("port");
        let token = info["token"].as_str().expect("token").to_string();
        Self {
            home,
            base: format!("http://127.0.0.1:{port}"),
            token,
            _guard: guard,
        }
    }

    /// The data directory this engine serves.
    pub fn data_dir(&self) -> std::path::PathBuf {
        self.home.path().join(".comemory")
    }

    /// Run a `comemory` subcommand against this engine's data directory.
    pub fn cli(&self, args: &[&str]) -> Value {
        let out = Command::new(cargo_bin("comemory"))
            .env("COMEMORY_DATA_DIR", self.data_dir())
            .arg("--json")
            .args(args)
            .output()
            .expect("run comemory");
        assert!(
            out.status.success(),
            "comemory {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).expect("json stdout")
    }

    /// A read against this engine's API.
    pub fn get(&self, path: &str) -> (u16, Value) {
        ureq_get(&format!("{}{path}", self.base), &self.token)
    }

    /// A write against this engine's API.
    pub fn post(&self, path: &str, body: &Value) -> (u16, Value) {
        ureq_post(&format!("{}{path}", self.base), &self.token, body)
    }

    /// Stop the server, keeping the data directory — what an operator does
    /// before a rebuild.
    pub fn stop(self) -> tempfile::TempDir {
        self.home
    }

    /// A stopped engine's data directory, usable for CLI commands that need
    /// the database to themselves.
    pub fn reopen(engine: Engine) -> StoppedEngine {
        StoppedEngine {
            home: engine.stop(),
        }
    }

    /// Open this engine's database directly, as an operator would.
    pub fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.data_dir().join("comemory.db")).expect("open db")
    }
}

/// Minimal blocking HTTP GET over the loopback API.
fn ureq_get(url: &str, token: &str) -> (u16, Value) {
    let response = reqwest::blocking::Client::new()
        .get(url)
        .bearer_auth(token)
        .send()
        .expect("GET");
    let status = response.status().as_u16();
    let body: Value = response.json().unwrap_or(Value::Null);
    (status, body)
}

/// Minimal blocking HTTP POST over the loopback API.
fn ureq_post(url: &str, token: &str, body: &Value) -> (u16, Value) {
    let response = reqwest::blocking::Client::new()
        .post(url)
        .bearer_auth(token)
        .json(body)
        .send()
        .expect("POST");
    let status = response.status().as_u16();
    let parsed: Value = response.json().unwrap_or(Value::Null);
    (status, parsed)
}

/// The payload of one memory on `engine`, as a peer would send it.
pub fn payload_of(engine: &Engine, id: &str) -> Value {
    let (status, body) = engine.get("/api/v1/sync/replica/changes?since=0&limit=50");
    assert_eq!(status, 200, "changes: {body}");
    let entries = body["data"]["entries"].as_array().expect("entries");
    let entry = entries
        .iter()
        .find(|e| e["entity_key"] == json!(id))
        .unwrap_or_else(|| panic!("no entry for {id}: {body}"));
    entry["payload"].clone()
}

/// The digest recorded for one memory's newest position on `engine`.
pub fn digest_of(engine: &Engine, id: &str) -> String {
    let (_, body) = engine.get("/api/v1/sync/replica/changes?since=0&limit=50");
    let entries = body["data"]["entries"].as_array().expect("entries").clone();
    entries
        .iter()
        .rev()
        .find(|e| e["entity_key"] == json!(id))
        .and_then(|e| e["payload_digest"].as_str())
        .expect("digest")
        .to_string()
}

/// An upsert envelope carrying `payload` under `operation_id`.
pub fn upsert_envelope(operation_id: &str, id: &str, digest: &str, payload: &Value) -> Value {
    json!({
        "protocol": "replica-v1",
        "operations": [{
            "operation_id": operation_id,
            "entity_kind": "memory",
            "entity_key": id,
            "op": "upsert",
            "schema_version": 1,
            "payload_digest": digest,
            "payload": payload,
        }]
    })
}

/// An engine whose server has been stopped: the data directory alone.
pub struct StoppedEngine {
    home: tempfile::TempDir,
}

impl StoppedEngine {
    /// The data directory.
    pub fn data_dir(&self) -> std::path::PathBuf {
        self.home.path().join(".comemory")
    }

    /// Run a `comemory` subcommand against it.
    pub fn cli(&self, args: &[&str]) -> Value {
        let out = Command::new(cargo_bin("comemory"))
            .env("COMEMORY_DATA_DIR", self.data_dir())
            .arg("--json")
            .args(args)
            .output()
            .expect("run comemory");
        assert!(
            out.status.success(),
            "comemory {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
    }

    /// Open its database directly.
    pub fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.data_dir().join("comemory.db")).expect("open db")
    }
}
