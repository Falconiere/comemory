//! End-to-end coverage of `comemory serve` against a real bound server: the
//! `--json` startup banner, token enforcement (401), path-traversal rejection
//! (403), and a full `GET`→`PUT`→`GET` edit round trip with stale-`If-Match`
//! conflict (409). This is where `router.rs`/`handlers.rs`/`mod.rs` glue is
//! exercised. Files are staged but not committed, so the fixture does not
//! depend on a working `git commit` (the read/write path needs only the
//! captured repo root + the on-disk files).

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};

use assert_cmd::cargo::cargo_bin;
use tempfile::TempDir;

use super::git_setup;

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

#[test]
fn serve_graph_token_traversal_and_edit_round_trip() {
    let home = TempDir::new().expect("home");
    let ws = TempDir::new().expect("workspace");
    let repo = ws.path().join("demo");
    git_setup::init_repo(&repo);
    std::fs::write(repo.join("a.rs"), "mod b;\nfn alpha() {}\n").expect("write a.rs");
    std::fs::write(repo.join("b.rs"), "fn beta() {}\n").expect("write b.rs");
    // Stage (no commit) so blob OIDs exist without needing `git commit`.
    git_setup::run_git(&repo, &["add", "-A"]);
    index_demo(&home, &repo);

    // Spawn the server on an ephemeral port and read its --json banner.
    let mut child = Command::new(cargo_bin("comemory"))
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args(["--json", "serve", "--repo", "demo", "--port", "0"])
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
    assert_eq!(info["read_only"], serde_json::Value::Bool(false));
    let base = format!("http://127.0.0.1:{port}");

    let client = reqwest::blocking::Client::new();

    // 1. /api/graph without the token → 401.
    let res = client
        .get(format!("{base}/api/graph"))
        .send()
        .expect("graph no token");
    assert_eq!(res.status().as_u16(), 401, "missing token must be 401");

    // 2. /api/graph with the token → 200 and our file nodes.
    let graph: serde_json::Value = client
        .get(format!("{base}/api/graph"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("graph")
        .json()
        .expect("graph json");
    let ids: Vec<&str> = graph["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .map(|n| n["id"].as_str().expect("id"))
        .collect();
    assert!(
        ids.contains(&"file:demo:a.rs"),
        "a.rs node present: {ids:?}"
    );

    // 3. Path-traversal id → 403.
    let res = client
        .get(format!("{base}/api/file?id=file:demo:../../etc/passwd"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("traversal");
    assert_eq!(res.status().as_u16(), 403, "traversal must be 403");

    // 4. Edit round trip: GET → PUT(If-Match) → GET, plus stale-match 409.
    let file: serde_json::Value = client
        .get(format!("{base}/api/file?id=file:demo:a.rs"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("get file")
        .json()
        .expect("file json");
    let old_oid = file["blob_oid"].as_str().expect("blob_oid").to_string();
    assert_eq!(file["lang"], "rust");
    assert_eq!(file["contents"], "mod b;\nfn alpha() {}\n");

    let new_body = "mod b;\nfn alpha() { let _x = 1; }\n";
    let put: serde_json::Value = client
        .put(format!("{base}/api/file?id=file:demo:a.rs"))
        .header("X-Comemory-Token", &token)
        .header("If-Match", &old_oid)
        .body(new_body)
        .send()
        .expect("put")
        .json()
        .expect("put json");
    let new_oid = put["blob_oid"].as_str().expect("new blob_oid").to_string();
    assert_ne!(new_oid, old_oid);

    // The edit landed on disk.
    assert_eq!(
        std::fs::read_to_string(repo.join("a.rs")).expect("read a.rs"),
        new_body
    );
    // A re-GET returns the new content + oid.
    let refetched: serde_json::Value = client
        .get(format!("{base}/api/file?id=file:demo:a.rs"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("re-get")
        .json()
        .expect("re-get json");
    assert_eq!(refetched["contents"], new_body);
    assert_eq!(refetched["blob_oid"], new_oid);

    // A stale If-Match (the now-superseded oid) → 409 Conflict.
    let res = client
        .put(format!("{base}/api/file?id=file:demo:a.rs"))
        .header("X-Comemory-Token", &token)
        .header("If-Match", &old_oid)
        .body("should not be written\n")
        .send()
        .expect("stale put");
    assert_eq!(res.status().as_u16(), 409, "stale If-Match must be 409");

    drop(guard);
}
