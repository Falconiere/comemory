//! End-to-end coverage of `GET|POST /api/v1/prune` and `POST /api/v1/gc`
//! (`src/serve/routes/maint/prune.rs`) against a real bound server. The
//! critical `GET` behavior under test: the route forces `apply = false`
//! unconditionally, so a `GET` — even one carrying `?apply=true` in the
//! query string — must never soft-delete anything (`api::prune::run` itself,
//! including the `apply: true` path, is covered directly in
//! `tests/api__prune.rs`; `api::gc::run` in `tests/api__gc.rs`).

#[path = "common/cli_prune_support.rs"]
mod support;

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use assert_cmd::cargo::cargo_bin;
use support::{make_prune_eligible, save_memory};
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

#[test]
fn v1_prune_returns_the_dry_run_report_envelope() {
    let home = TempDir::new().expect("home");
    let id = save_memory(&home, "stale prune candidate body via http");
    make_prune_eligible(&home, &id);
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/prune"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 prune");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["meta"]["command"], "prune");
    let items = body["data"]["low_value_memories"]["items"]
        .as_array()
        .expect("items array");
    assert!(
        items.iter().any(|v| v == &serde_json::json!(id)),
        "expected {id} in low_value_memories items: {items:?}"
    );
    assert_eq!(
        body["data"]["low_value_memories"]["total"].as_u64(),
        Some(1)
    );
}

#[test]
fn v1_prune_ignores_apply_true_in_the_query_string() {
    let home = TempDir::new().expect("home");
    let id = save_memory(&home, "doomed-looking but must survive the GET");
    make_prune_eligible(&home, &id);
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/prune?apply=true"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 prune apply=true");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));

    // A GET must never soft-delete, regardless of ?apply=true: re-running the
    // report must still find the same candidate, and its markdown must still
    // live in memories/ (not memories/.trash/).
    let follow_up = client
        .get(format!("{base}/api/v1/prune"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 prune follow-up");
    let follow_up_body: serde_json::Value = follow_up.json().expect("json");
    assert_eq!(
        follow_up_body["data"]["low_value_memories"]["total"].as_u64(),
        Some(1),
        "candidate must still be reported after a GET with ?apply=true"
    );

    let memories_dir = home.path().join(".comemory").join("memories");
    let still_live = std::fs::read_dir(&memories_dir)
        .expect("read memories dir")
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with(id.as_str()))
        .count();
    assert_eq!(still_live, 1, "{id} must remain in memories/, not .trash/");
}

#[test]
fn v1_prune_post_without_confirm_is_400_confirmation_required() {
    let home = TempDir::new().expect("home");
    let id = save_memory(&home, "must survive an unconfirmed POST /prune");
    make_prune_eligible(&home, &id);
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/prune"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({ "apply": true }))
        .send()
        .expect("v1 prune post no confirm");
    assert_eq!(res.status().as_u16(), 400);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "confirmation_required");

    let memories_dir = home.path().join(".comemory").join("memories");
    let still_live = std::fs::read_dir(&memories_dir)
        .expect("read memories dir")
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with(id.as_str()))
        .count();
    assert_eq!(still_live, 1, "{id} must survive a missing-confirm POST");
}

#[test]
fn v1_prune_post_on_a_read_only_server_is_405_before_confirm_is_checked() {
    let home = TempDir::new().expect("home");
    let id = save_memory(&home, "read-only server must 405 before confirm");
    make_prune_eligible(&home, &id);
    let (base, token, _guard) = spawn_serve(&home, &["--read-only"]);
    let client = reqwest::blocking::Client::new();

    // No `confirm` in the body either — read-only must still win (AC-19).
    let res = client
        .post(format!("{base}/api/v1/prune"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({ "apply": true }))
        .send()
        .expect("v1 prune post read-only");
    assert_eq!(res.status().as_u16(), 405);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "read_only");
}

#[test]
fn v1_prune_post_confirmed_applies_the_cleanup() {
    let home = TempDir::new().expect("home");
    let id = save_memory(&home, "doomed via a confirmed POST /prune");
    make_prune_eligible(&home, &id);
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/prune"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({ "apply": true, "confirm": true }))
        .send()
        .expect("v1 prune post confirmed");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));

    let memories_dir = home.path().join(".comemory").join("memories");
    let still_live = std::fs::read_dir(&memories_dir)
        .expect("read memories dir")
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with(id.as_str()))
        .count();
    assert_eq!(still_live, 0, "{id} must have moved to .trash/");
    let trashed = std::fs::read_dir(memories_dir.join(".trash"))
        .expect("read .trash")
        .count();
    assert_eq!(trashed, 1);
}

#[test]
fn v1_gc_post_without_confirm_is_400_confirmation_required() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/gc"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({}))
        .send()
        .expect("v1 gc post no confirm");
    assert_eq!(res.status().as_u16(), 400);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "confirmation_required");
}

#[test]
fn v1_gc_post_on_a_read_only_server_is_405() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &["--read-only"]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/gc"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({ "confirm": true }))
        .send()
        .expect("v1 gc post read-only");
    assert_eq!(res.status().as_u16(), 405);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "read_only");
}

#[test]
fn v1_gc_post_confirmed_returns_the_removal_envelope() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/gc"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({ "confirm": true }))
        .send()
        .expect("v1 gc post confirmed");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["meta"]["command"], "gc");
    assert!(body["data"]["removed"].is_u64(), "body: {body}");
}
