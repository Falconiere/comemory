//! End-to-end coverage of `GET|POST /api/v1/memories/search` and
//! `GET|POST /api/v1/context` (`src/serve/routes/memories/search.rs`)
//! against a real bound server, seeded via real `comemory save` calls.
//!
//! Covers AC-2 (HTTP search hit ids/order match `comemory search --json`,
//! access tracking disabled on both sides so the first call cannot reorder
//! the second) and AC-3 (a wrong-dimension `vector` is a `422
//! vec_dim_mismatch` envelope; a well-formed 1024-dim vector still
//! succeeds, paged).

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use assert_cmd::Command as AssertCommand;
use assert_cmd::cargo::cargo_bin;
use tempfile::TempDir;

#[path = "common/vectors.rs"]
mod vectors;

/// Kills the spawned server on drop so a panicking assertion cannot leak it.
struct ServerGuard(Child);
impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Save a memory through the real binary.
fn save(home: &TempDir, body: &str, kind: &str, repo: &str) {
    AssertCommand::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args(["save", body, "--kind", kind, "--repo", repo])
        .assert()
        .success();
}

/// Spawn `comemory serve` on an ephemeral port with
/// `COMEMORY_DISABLE_ACCESS_TRACKING=true` set (matching the CLI-side env
/// used for the AC-2 comparison), returning the base URL, the session
/// token, and the kill-on-drop guard.
fn spawn_serve(home: &TempDir) -> (String, String, ServerGuard) {
    let mut child = Command::new(cargo_bin("comemory"))
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .env("COMEMORY_DISABLE_ACCESS_TRACKING", "true")
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

/// Run `comemory search <query> --json --k <k>` against `home`, with the
/// same access-tracking-disabled env the server was spawned with, and
/// return the ordered `hits[].memory_id` list.
fn cli_search_ids(home: &TempDir, query: &str, k: usize) -> Vec<String> {
    let out = AssertCommand::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .env("COMEMORY_DISABLE_ACCESS_TRACKING", "true")
        .args(["--json", "search", query, "--k", &k.to_string()])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("search --json");
    v["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .map(|h| h["memory_id"].as_str().expect("memory_id").to_string())
        .collect()
}

fn seeded_home() -> TempDir {
    let home = TempDir::new().expect("home");
    save(
        &home,
        "postgres pool exhausted under load spikes",
        "bug",
        "demo",
    );
    save(
        &home,
        "redis cache eviction policy set to allkeys-lru",
        "decision",
        "demo",
    );
    save(
        &home,
        "advisory lock guards the migration runner",
        "pattern",
        "demo",
    );
    home
}

#[test]
fn v1_search_hit_order_matches_the_cli_json_output() {
    let home = seeded_home();
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!(
            "{base}/api/v1/memories/search?query=postgres+pool&k=12"
        ))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 search");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("v1 search json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["meta"]["command"], "search");
    let http_ids: Vec<String> = body["data"]["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .map(|h| h["memory_id"].as_str().expect("memory_id").to_string())
        .collect();
    assert!(!http_ids.is_empty(), "body: {body}");

    let cli_ids = cli_search_ids(&home, "postgres pool", 12);
    assert_eq!(
        http_ids, cli_ids,
        "HTTP and CLI hit order must match (AC-2)"
    );
}

#[test]
fn v1_search_post_wrong_dim_vector_is_422_vec_dim_mismatch() {
    let home = seeded_home();
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/memories/search"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({ "query": "postgres", "vector": [0.1, 0.2, 0.3] }))
        .send()
        .expect("v1 search wrong-dim vector");
    assert_eq!(res.status().as_u16(), 422);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(false));
    assert_eq!(body["error"]["code"], "vec_dim_mismatch");
}

#[test]
fn v1_search_post_well_formed_vector_succeeds_paged() {
    let home = seeded_home();
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let vector = vectors::vector("v1-search-ac3", 1024);
    let res = client
        .post(format!("{base}/api/v1/memories/search"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({ "query": "postgres", "vector": vector }))
        .send()
        .expect("v1 search well-formed vector");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true), "body: {body}");
    assert!(body["data"]["hits"].is_array(), "body: {body}");
    assert!(body["data"]["limit"].is_number(), "body: {body}");
}

#[test]
fn v1_context_get_returns_a_bundle_envelope() {
    let home = seeded_home();
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/context?query=postgres+pool"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 context");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["meta"]["command"], "context");
    assert_eq!(body["data"]["query"], "postgres pool");
    assert!(body["data"]["memories"].is_array(), "body: {body}");
}
