#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! End-to-end coverage of `GET /api/v1/graph` and `GET /api/v1/edges`
//! (`src/serve/routes/graph.rs`) against a real bound server, plus a legacy
//! `GET /api/graph` byte-compat check proving the v1 route added no second
//! query path.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use assert_cmd::Command as AssertCommand;
use assert_cmd::cargo::cargo_bin;
use tempfile::TempDir;

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

/// Kills the spawned server on drop so a panicking assertion cannot leak it.
struct ServerGuard(Child);
impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Index a two-file repo with one static import into `home`'s data dir via
/// the real binary; returns the fixture workspace kept alive for the test.
fn seeded_home() -> (TempDir, TempDir) {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let repo = workspace.path().join("import-repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(
        &repo,
        &[
            ("main.rs", "mod parser;\n\nfn main() {}\n"),
            ("parser.rs", "fn parse() {}\n"),
        ],
        "seed the import pair",
    );
    AssertCommand::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args(["index-code", "--repo", "demo", "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();
    (home, workspace)
}

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
fn v1_graph_without_window_returns_the_full_envelope() {
    let (home, _workspace) = seeded_home();
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/graph"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 graph");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["meta"]["command"], "graph");
    assert!(body["data"]["nodes"].is_array(), "body: {body}");
    assert!(body["data"]["edges"].is_array(), "body: {body}");
    // No pagination cursor on the full-graph shape.
    assert!(body["data"]["limit"].is_null(), "body: {body}");
}

#[test]
fn v1_graph_with_limit_returns_a_page_envelope() {
    let (home, _workspace) = seeded_home();
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/graph?limit=1"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 graph page");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["data"]["limit"], 1);
    assert_eq!(body["data"]["offset"], 0);
    assert!(body["data"]["total"].is_number(), "body: {body}");
    assert!(body["data"]["has_more"].is_boolean(), "body: {body}");
}

/// Save `body` via the real binary, returning the assigned memory id.
fn save_id(home: &TempDir, body: &str, kind: &str, repo: &str) -> String {
    let out = AssertCommand::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args(["--json", "save", body, "--kind", kind, "--repo", repo])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    serde_json::from_str::<serde_json::Value>(stdout.trim()).expect("save json")["id"]
        .as_str()
        .expect("id")
        .to_string()
}

/// Seed a real `queue design v1` → `v2` supersede relation into `home`.
fn seed_supersede(home: &TempDir) {
    let old = save_id(home, "queue design v1", "decision", "demo");
    AssertCommand::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args([
            "save",
            "queue design v2",
            "--kind",
            "decision",
            "--repo",
            "demo",
            "--supersedes",
            &old,
        ])
        .assert()
        .success();
}

#[test]
fn v1_edges_returns_the_matched_triplet_page() {
    let home = TempDir::new().expect("home");
    seed_supersede(&home);

    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();
    let res = client
        .get(format!("{base}/api/v1/edges?query=supersedes+queue"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 edges");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["meta"]["command"], "edges");
    let items = body["data"]["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1, "body: {body}");
    assert_eq!(items[0]["rel"], "supersedes");
}

#[test]
fn legacy_api_graph_stays_byte_compatible_with_no_params() {
    let (home, _workspace) = seeded_home();
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/graph"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("legacy graph");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    // Legacy shape: bare `{nodes, edges}`, no envelope wrapper.
    assert!(
        body.get("ok").is_none(),
        "legacy body must be unenveloped: {body}"
    );
    assert!(body["nodes"].is_array(), "body: {body}");
    assert!(body["edges"].is_array(), "body: {body}");
}
