//! End-to-end coverage of `GET /api/search` against a real bound server over a
//! real temp `index-code` index: a lexical query returns file hits whose
//! `node_id`s intersect `GET /api/graph`'s node ids (`mode == "lexical"`), a
//! garbage query returns `200` with empty hits, and a failing `--embed-cmd`
//! DEGRADES to a `200` lexical response (never a 5xx). Mirrors the
//! `cli__serve.rs` spawn/banner/authed-request harness.

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

/// Spawn `comemory serve` on an ephemeral port with the given extra args,
/// returning the base URL, the session token, and the kill-on-drop guard.
fn spawn_serve(home: &TempDir, extra: &[&str]) -> (String, String, ServerGuard) {
    let mut args = vec!["--json", "serve", "--repo", "demo", "--port", "0"];
    args.extend_from_slice(extra);
    let mut child = Command::new(cargo_bin("comemory"))
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args(&args)
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

/// Build a small real repo, index it, and run the lexical + empty + degrade
/// assertions. One test so the (slow) index + spawn happens once.
#[test]
fn serve_search_lexical_empty_and_embed_degrade() {
    let home = TempDir::new().expect("home");
    let ws = TempDir::new().expect("workspace");
    let repo = ws.path().join("demo");
    git_repo::init_repo(&repo);
    std::fs::write(
        repo.join("router.rs"),
        "fn build_router() {}\nfn route_request() {}\n",
    )
    .expect("write router.rs");
    std::fs::write(repo.join("other.rs"), "fn unrelated_thing() {}\n").expect("write other.rs");
    git_repo::run_git(&repo, &["add", "-A"]);
    index_demo(&home, &repo);

    let client = reqwest::blocking::Client::new();

    let (base, token, guard) = spawn_serve(&home, &[]);
    assert_lexical_hits(&client, &base, &token);
    assert_empty_variants(&client, &base, &token);
    drop(guard);

    let (base, token, guard) = spawn_serve(&home, &["--embed-cmd", "exit 1"]);
    assert_embed_degrade(&client, &base, &token);
    drop(guard);

    // Embed SUCCEEDS but returns a wrong-dimension vector: `cat` drains the
    // query off stdin so the embed shell-out completes, then emits a 2-float
    // payload. The vec0 dim guard rejects it inside retrieval, which must take
    // the lexical-retry branch (not surface a 5xx).
    let (base, token, guard) = spawn_serve(
        &home,
        &[
            "--embed-cmd",
            "cat >/dev/null; printf '{\"embedding\":[0.1,0.2]}'",
        ],
    );
    assert_embed_degrade(&client, &base, &token);
    drop(guard);
}

/// A lexical query (no embed-cmd) returns `mode == "lexical"` with non-empty
/// hits, every `node_id` is a `file:` id, and each maps 1:1 onto a
/// `/api/graph` node id. Also checks token enforcement on the new route.
fn assert_lexical_hits(client: &reqwest::blocking::Client, base: &str, token: &str) {
    let no_token = client
        .get(format!("{base}/api/search?q=router"))
        .send()
        .expect("search no token");
    assert_eq!(no_token.status().as_u16(), 401, "missing token must be 401");

    let res: serde_json::Value = client
        .get(format!("{base}/api/search?q=build_router"))
        .header("X-Comemory-Token", token)
        .send()
        .expect("search")
        .json()
        .expect("search json");
    assert_eq!(res["mode"], "lexical", "no embed-cmd → lexical mode");
    let hits = res["hits"].as_array().expect("hits array");
    assert!(!hits.is_empty(), "a matching lexical query returns hits");

    let graph_ids = graph_node_ids(client, base, token);
    for h in hits {
        let node_id = h["node_id"].as_str().expect("node_id");
        assert!(
            node_id.starts_with("file:"),
            "node_id is a file graph id: {node_id}"
        );
        assert!(
            graph_ids.contains(node_id),
            "hit {node_id} is a real graph node; graph ids: {graph_ids:?}"
        );
    }
}

/// Both a no-match query and an empty query return `200` with empty hits.
fn assert_empty_variants(client: &reqwest::blocking::Client, base: &str, token: &str) {
    for q in ["zzzznomatchatallqqq", ""] {
        let res: serde_json::Value = client
            .get(format!("{base}/api/search?q={q}"))
            .header("X-Comemory-Token", token)
            .send()
            .expect("search")
            .json()
            .expect("search json");
        assert!(
            res["hits"].as_array().expect("hits").is_empty(),
            "query {q:?} returns empty hits"
        );
    }
}

/// A failing `--embed-cmd` must degrade to a `200` lexical response (with the
/// matching hits) rather than surfacing the embed error as a 5xx.
fn assert_embed_degrade(client: &reqwest::blocking::Client, base: &str, token: &str) {
    let degraded = client
        .get(format!("{base}/api/search?q=build_router"))
        .header("X-Comemory-Token", token)
        .send()
        .expect("degraded search");
    assert_eq!(
        degraded.status().as_u16(),
        200,
        "embed-cmd failure must degrade to 200, not 5xx"
    );
    let body: serde_json::Value = degraded.json().expect("degraded json");
    assert_eq!(
        body["mode"], "lexical",
        "failed embed-cmd falls back to lexical mode"
    );
    assert!(
        !body["hits"].as_array().expect("hits").is_empty(),
        "the lexical fallback still returns the matching hits"
    );
}

/// Fetch the full `/api/graph` and collect its node ids into a set.
fn graph_node_ids(
    client: &reqwest::blocking::Client,
    base: &str,
    token: &str,
) -> std::collections::BTreeSet<String> {
    let graph: serde_json::Value = client
        .get(format!("{base}/api/graph"))
        .header("X-Comemory-Token", token)
        .send()
        .expect("graph")
        .json()
        .expect("graph json");
    graph["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .map(|n| n["id"].as_str().expect("id").to_string())
        .collect()
}
