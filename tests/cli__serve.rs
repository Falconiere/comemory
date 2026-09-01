#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! End-to-end coverage of `comemory serve` against a real bound server: the
//! `--json` startup banner, token enforcement (401), the `/api/v1/graph`
//! full-vs-paged switch over a really indexed repo, the `--repo` default
//! scope, and the `--read-only` gate (405). This is where `cli::serve` /
//! `serve::serve` / `router.rs` glue is exercised end to end; the
//! per-route behavior is covered in-process by `tests/common/serve_state.rs`
//! suites. Files are staged but not committed, so the fixture does not
//! depend on a working `git commit`.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};

use assert_cmd::cargo::cargo_bin;
use tempfile::TempDir;

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

/// Run `index-code` for the `demo` repo to completion.
fn index_demo(home: &TempDir, repo: &Path) {
    assert_cmd::Command::cargo_bin("comemory")
        .unwrap()
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args(["index-code", "--repo", "demo", "--path"])
        .arg(repo)
        .assert()
        .success();
}

/// Spawn `comemory serve --json` with `extra_args`, returning the base URL,
/// the session token, and the kill-on-drop guard.
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
    assert_eq!(token.len(), 64, "token is 64 hex chars");
    assert_eq!(
        info["url"],
        serde_json::Value::String(format!("http://127.0.0.1:{port}/api/v1")),
        "the banner names the API base, never a page"
    );
    (format!("http://127.0.0.1:{port}"), token, guard)
}

/// A staged (uncommitted) two-file repo with one import edge candidate.
fn stage_demo_repo(ws: &TempDir) -> std::path::PathBuf {
    let repo = ws.path().join("demo");
    git_repo::init_repo(&repo);
    std::fs::write(repo.join("a.rs"), "mod b;\nfn alpha() {}\n").expect("write a.rs");
    std::fs::write(repo.join("b.rs"), "fn beta() {}\n").expect("write b.rs");
    git_repo::run_git(&repo, &["add", "-A"]);
    repo
}

#[test]
fn serve_banner_token_gate_and_v1_graph_over_a_real_index() {
    let home = TempDir::new().expect("home");
    let ws = TempDir::new().expect("workspace");
    let repo = stage_demo_repo(&ws);
    index_demo(&home, &repo);
    let (base, token, _guard) = spawn_serve(&home, &["--repo", "demo"]);
    let client = reqwest::blocking::Client::new();

    // 1. No token → the enveloped 401.
    let res = client
        .get(format!("{base}/api/v1/graph"))
        .send()
        .expect("graph no token");
    assert_eq!(res.status().as_u16(), 401, "missing token must be 401");
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "unauthorized");

    // 2. Bearer form → 200, the full graph, our file nodes present.
    let body: serde_json::Value = client
        .get(format!("{base}/api/v1/graph"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .expect("graph")
        .json()
        .expect("graph json");
    assert_eq!(body["ok"], serde_json::json!(true), "body: {body}");
    let ids: Vec<&str> = body["data"]["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .map(|n| n["id"].as_str().expect("id"))
        .collect();
    assert!(
        ids.contains(&"file:demo:a.rs"),
        "a.rs node present: {ids:?}"
    );
    assert!(
        body["data"].get("total").is_none() && body["data"].get("has_more").is_none(),
        "no-param /graph stays the full graph (no pagination envelope)"
    );

    // 3. The paged envelope, and `--repo demo` as the default scope: a
    //    header naming an unknown repo narrows the graph to nothing.
    let page: serde_json::Value = client
        .get(format!("{base}/api/v1/graph?limit=1&offset=0"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("graph page")
        .json()
        .expect("graph page json");
    assert_eq!(page["data"]["limit"], 1, "envelope echoes the window");
    let total = page["data"]["total"].as_u64().expect("total");
    let shown = page["data"]["edges"].as_array().expect("edges").len() as u64;
    assert!(shown <= 1, "the window holds at most `limit` edges");
    assert_eq!(page["data"]["has_more"], serde_json::json!(shown < total));

    let scoped: serde_json::Value = client
        .get(format!("{base}/api/v1/graph"))
        .header("X-Comemory-Token", &token)
        .header("X-Comemory-Repo", "no-such-repo")
        .send()
        .expect("scoped graph")
        .json()
        .expect("scoped json");
    assert!(
        scoped["data"]["nodes"]
            .as_array()
            .expect("nodes")
            .is_empty(),
        "X-Comemory-Repo overrides the --repo default: {scoped}"
    );
}

#[test]
fn serve_read_only_refuses_a_mutating_route_with_405() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &["--read-only"]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/memories"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({ "body": "never written", "kind": "note" }))
        .send()
        .expect("save on read-only");
    assert_eq!(res.status().as_u16(), 405);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "read_only");
    // Not "the directory is absent OR holds nothing": the server calls
    // `ensure_dirs` at startup, so an absent directory would mean the
    // server never came up and the assertion would pass for the wrong
    // reason. It must exist, and it must hold no memory file.
    let memories = home.path().join(".comemory").join("memories");
    assert!(
        memories.is_dir(),
        "the server creates its data dirs at startup, read-only or not"
    );
    let written: Vec<String> = std::fs::read_dir(&memories)
        .expect("read memories dir")
        .filter_map(std::result::Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with('.'))
        .collect();
    assert!(
        written.is_empty(),
        "a read-only server must write no memory file, found {written:?}"
    );

    let health: serde_json::Value = client
        .get(format!("{base}/api/v1/health"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("health")
        .json()
        .expect("health json");
    assert_eq!(health["data"]["read_only"], serde_json::json!(true));
}
