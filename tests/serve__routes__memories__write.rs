//! End-to-end coverage of the mutating memory routes
//! (`src/serve/routes/memories/write.rs`) against a real bound server:
//! `POST /api/v1/memories` (AC-1), `DELETE /api/v1/memories/{id}?confirm=`
//! (AC-5), `POST /api/v1/feedback`, and the read-only 405 sweep. Mirrors
//! the `ServerGuard`/`spawn_serve` harness in
//! `tests/serve__routes__maint__mod.rs`.

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

/// Run `comemory list --json` against `home`'s data dir, returning the
/// decoded JSON array.
fn cli_list(home: &TempDir) -> Vec<serde_json::Value> {
    let out = AssertCommand::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args(["--json", "list"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    let body: serde_json::Value = serde_json::from_str(stdout.trim()).expect("list json");
    body["items"]
        .as_array()
        .expect("items array")
        .clone()
        .into_iter()
        .collect()
}

#[test]
fn v1_post_memories_saves_via_the_shared_store_ac1() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/memories"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({
            "body": "postgres advisory lock ordering fix for the migration runner",
            "kind": "decision",
            "tags": ["db", "pg"],
        }))
        .send()
        .expect("post memories");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["meta"]["command"], "save");
    let id = body["data"]["id"].as_str().expect("id").to_string();
    let path = body["data"]["path"].as_str().expect("path").to_string();
    assert!(
        std::path::Path::new(&path).exists(),
        "markdown file should exist at {path}"
    );

    // Prove the two surfaces share one store: a fresh CLI process on the
    // same data dir sees the same id.
    let items = cli_list(&home);
    assert!(
        items.iter().any(|it| it["id"] == id),
        "comemory list --json must show the HTTP-saved id {id}: {items:?}"
    );
}

/// Save one memory through the real binary, returning its id.
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
fn v1_delete_memories_id_requires_confirm_then_soft_deletes_ac5() {
    let home = TempDir::new().expect("home");
    let id = save_id(&home, "a memory that will be deleted over http");
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    // Without `?confirm=true`: 400 confirmation_required, memory survives.
    let res = client
        .delete(format!("{base}/api/v1/memories/{id}"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("delete without confirm");
    assert_eq!(res.status().as_u16(), 400);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "confirmation_required");

    let get_res = client
        .get(format!("{base}/api/v1/memories/{id}"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("get after unconfirmed delete");
    assert_eq!(get_res.status().as_u16(), 200, "memory must still exist");

    // With `?confirm=true`: soft-deletes, file moves to .trash/, GET → 404.
    let del_res = client
        .delete(format!("{base}/api/v1/memories/{id}?confirm=true"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("confirmed delete");
    assert_eq!(del_res.status().as_u16(), 200);
    let del_body: serde_json::Value = del_res.json().expect("json");
    assert_eq!(del_body["data"]["deleted"], id);

    let trash = home.path().join(".comemory/memories/.trash");
    let moved = std::fs::read_dir(&trash)
        .expect("read trash dir")
        .filter_map(|e| e.ok())
        .any(|e| e.file_name().to_string_lossy().starts_with(&id));
    assert!(moved, "markdown file should have moved into .trash/");

    let get_after = client
        .get(format!("{base}/api/v1/memories/{id}"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("get after confirmed delete");
    assert_eq!(get_after.status().as_u16(), 404);
    let after_body: serde_json::Value = get_after.json().expect("json");
    assert_eq!(after_body["error"]["code"], "not_found");
}

#[test]
fn v1_post_feedback_records_against_an_unlogged_query_id() {
    let home = TempDir::new().expect("home");
    let id = save_id(&home, "a memory used to test http feedback recording");
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/feedback"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({
            "query_id": "q-20260610-a1b2c3d4",
            "used": [id],
        }))
        .send()
        .expect("post feedback");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["meta"]["command"], "feedback");
    assert_eq!(body["data"]["used"].as_u64(), Some(1));
    assert_eq!(body["data"]["query_id"], "q-20260610-a1b2c3d4");
    // Never logged via a real `comemory search --json` call in this test —
    // `known_query` is honestly `false`, which is still a real assertion.
    assert_eq!(body["data"]["known_query"], serde_json::json!(false));
}

#[test]
fn v1_read_only_server_405s_every_mutating_memory_route() {
    let home = TempDir::new().expect("home");
    let id = save_id(&home, "a memory protected by a read-only server");
    let (base, token, _guard) = spawn_serve(&home, &["--read-only"]);
    let client = reqwest::blocking::Client::new();

    let save_res = client
        .post(format!("{base}/api/v1/memories"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({"body": "rejected by a read-only server"}))
        .send()
        .expect("post memories read-only");
    assert_eq!(save_res.status().as_u16(), 405);
    let save_body: serde_json::Value = save_res.json().expect("json");
    assert_eq!(save_body["error"]["code"], "read_only");

    // Read-only outranks even a confirmed delete (AC-19).
    let del_res = client
        .delete(format!("{base}/api/v1/memories/{id}?confirm=true"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("delete read-only");
    assert_eq!(del_res.status().as_u16(), 405);
    let del_body: serde_json::Value = del_res.json().expect("json");
    assert_eq!(del_body["error"]["code"], "read_only");

    let feedback_res = client
        .post(format!("{base}/api/v1/feedback"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({"query_id": "q-20260610-a1b2c3d4", "used": [id]}))
        .send()
        .expect("post feedback read-only");
    assert_eq!(feedback_res.status().as_u16(), 405);
}
