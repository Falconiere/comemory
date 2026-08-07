#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! End-to-end coverage of `GET /api/v1/jobs`, `GET /api/v1/jobs/{id}` and
//! `GET /api/v1/jobs/{id}/events` (`src/serve/routes/jobs.rs`) against a
//! real bound server.
//!
//! No route creates a job yet (`POST /code/index` and friends land in
//! s3.2), so what a live server can honestly prove here is the empty-list
//! shape, the unknown-id `404` on all three routes — the SSE route included,
//! which answers with the same enveloped `not_found` rather than opening a
//! stream — and the `?token=`-only auth path `EventSource` needs (AC-11).
//! The job lifecycle itself, including AC-8's watch-replay mechanism, is
//! proven directly in `tests/serve__jobs__{registry,worker}.rs`.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use assert_cmd::cargo::cargo_bin;
use tempfile::TempDir;

/// A syntactically valid job id (16 lowercase hex) that was never issued.
const UNKNOWN_ID: &str = "0123456789abcdef";

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
fn v1_jobs_list_is_an_empty_page_on_a_fresh_server() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/jobs"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 jobs");

    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["meta"]["command"], "jobs.list");
    assert_eq!(body["data"]["items"], serde_json::json!([]));
    assert_eq!(body["data"]["total"].as_u64(), Some(0));
    assert_eq!(body["data"]["has_more"], serde_json::json!(false));
    assert_eq!(body["data"]["limit"].as_u64(), Some(50));
}

#[test]
fn v1_jobs_get_on_an_unknown_id_is_an_enveloped_404() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/jobs/{UNKNOWN_ID}"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 jobs/{id}");

    assert_eq!(res.status().as_u16(), 404);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(false));
    assert_eq!(body["error"]["code"], "not_found");
    assert_eq!(body["meta"]["command"], "jobs.get");
}

/// The SSE route answers an unknown id with the same enveloped `404` the
/// other job routes use, instead of opening a stream that could never emit
/// — a client cannot confuse "no such job" with "a silent job".
#[test]
fn v1_jobs_events_on_an_unknown_id_is_an_enveloped_404_not_a_stream() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/jobs/{UNKNOWN_ID}/events"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 jobs/{id}/events");

    assert_eq!(res.status().as_u16(), 404);
    let content_type = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.starts_with("application/json"),
        "unknown-id SSE reply must be the JSON envelope, got {content_type}"
    );
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(false));
    assert_eq!(body["error"]["code"], "not_found");
    assert_eq!(body["meta"]["command"], "jobs.events");
}

/// AC-11's SSE-specific claim: `EventSource` cannot send headers, so the
/// `?token=` query form alone must pass the router guard. The job itself is
/// unknown (404) — the point is that the guard's 401 did *not* fire.
#[test]
fn v1_jobs_events_authenticates_with_the_token_query_param_alone() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!(
            "{base}/api/v1/jobs/{UNKNOWN_ID}/events?token={token}"
        ))
        .send()
        .expect("v1 jobs events with ?token=");

    assert_eq!(
        res.status().as_u16(),
        404,
        "?token= alone must authenticate the SSE path"
    );
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "not_found");
}

#[test]
fn v1_jobs_events_without_a_token_is_an_enveloped_401() {
    let home = TempDir::new().expect("home");
    let (base, _token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    for url in [
        format!("{base}/api/v1/jobs/{UNKNOWN_ID}/events"),
        format!("{base}/api/v1/jobs/{UNKNOWN_ID}/events?token=deadbeef"),
        format!("{base}/api/v1/jobs"),
    ] {
        let res = client.get(&url).send().expect("unauthenticated request");
        assert_eq!(res.status().as_u16(), 401, "url: {url}");
        let body: serde_json::Value = res.json().expect("json");
        assert_eq!(body["ok"], serde_json::json!(false), "url: {url}");
        assert_eq!(body["error"]["code"], "unauthorized", "url: {url}");
    }
}
