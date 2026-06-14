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

#[test]
fn serve_graph_token_traversal_and_edit_round_trip() {
    let home = TempDir::new().expect("home");
    let ws = TempDir::new().expect("workspace");
    let repo = ws.path().join("demo");
    git_repo::init_repo(&repo);
    std::fs::write(repo.join("a.rs"), "mod b;\nfn alpha() {}\n").expect("write a.rs");
    std::fs::write(repo.join("b.rs"), "fn beta() {}\n").expect("write b.rs");
    // Stage (no commit) so blob OIDs exist without needing `git commit`.
    git_repo::run_git(&repo, &["add", "-A"]);
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
    // Back-compat: with no pagination params the response is the bare
    // `{nodes, edges}` graph — no envelope cursor fields leak in.
    assert!(
        graph.get("total").is_none() && graph.get("has_more").is_none(),
        "no-param /api/graph stays the full graph (no pagination envelope)"
    );

    // 2b/2c. The paginated envelope + the defensive negative-param 400.
    assert_graph_pagination(&client, &base, &token);

    // 3. Path-traversal id → 403.
    let res = client
        .get(format!("{base}/api/file?id=file:demo:../../etc/passwd"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("traversal");
    assert_eq!(res.status().as_u16(), 403, "traversal must be 403");

    // 4 & 5. Edit round trip: GET → PUT(If-Match) → GET, a 3 MiB save, and the
    // stale-match 409.
    assert_edit_round_trip(&client, &base, &token, &repo);

    drop(guard);
}

/// Exercise the `?limit=&offset=` pagination of `GET /api/graph`: the
/// `GraphPage` envelope (window echo, an `edges` window bounded by `limit` and
/// an exact `has_more` derived from `total`, nodes derived from only the
/// windowed edges) and the defensive negative-param 400. The serve fixture
/// stages files without committing, so it has no mined edges — the assertions
/// therefore hold for any `total >= 0` rather than a fixed edge count.
/// Extracted from the round-trip test to keep each function focused.
fn assert_graph_pagination(client: &reqwest::blocking::Client, base: &str, token: &str) {
    let page: serde_json::Value = client
        .get(format!("{base}/api/graph?limit=1&offset=0"))
        .header("X-Comemory-Token", token)
        .send()
        .expect("graph page")
        .json()
        .expect("graph page json");
    // The presence of these cursor fields proves we got the paginated envelope,
    // not the bare back-compat `{nodes, edges}` graph.
    assert_eq!(page["limit"], 1, "envelope echoes the window");
    assert_eq!(page["offset"], 0);
    let total = page["total"].as_u64().expect("total");
    let shown = page["edges"].as_array().expect("edges").len() as u64;
    assert!(shown <= 1, "the window holds at most `limit` edges");
    assert_eq!(
        page["has_more"],
        serde_json::Value::Bool(shown < total),
        "has_more is exact against the full edge count"
    );
    // Derived nodes: every node is an endpoint of a windowed edge (so an empty
    // edge window yields no nodes — no full-graph node dump leaks through).
    let page_nodes: std::collections::BTreeSet<&str> = page["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .map(|n| n["id"].as_str().expect("id"))
        .collect();
    let endpoints: std::collections::BTreeSet<&str> = page["edges"]
        .as_array()
        .expect("edges")
        .iter()
        .flat_map(|e| {
            [
                e["src"].as_str().expect("src"),
                e["dst"].as_str().expect("dst"),
            ]
        })
        .collect();
    assert_eq!(
        page_nodes, endpoints,
        "page nodes are exactly the windowed edges' endpoints"
    );

    // A negative window param is rejected with 400 (defensive parse).
    let bad = client
        .get(format!("{base}/api/graph?limit=-1"))
        .header("X-Comemory-Token", token)
        .send()
        .expect("bad limit");
    assert_eq!(bad.status().as_u16(), 400, "negative limit must be 400");
}

/// Drive the editor round trip: GET → PUT(If-Match) → re-GET, a 3 MiB save
/// (above axum's 2 MiB default body limit, below the 5 MiB editor cap), and a
/// stale-`If-Match` 409. Extracted to keep the bound-server test focused.
fn assert_edit_round_trip(
    client: &reqwest::blocking::Client,
    base: &str,
    token: &str,
    repo: &Path,
) {
    let file: serde_json::Value = client
        .get(format!("{base}/api/file?id=file:demo:a.rs"))
        .header("X-Comemory-Token", token)
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
        .header("X-Comemory-Token", token)
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
        .header("X-Comemory-Token", token)
        .send()
        .expect("re-get")
        .json()
        .expect("re-get json");
    assert_eq!(refetched["contents"], new_body);
    assert_eq!(refetched["blob_oid"], new_oid);

    // A 3 MiB save must reach our handler and succeed, not hit a generic 413.
    let big = format!("// big\n{}", "a".repeat(3 * 1024 * 1024));
    let big_put = client
        .put(format!("{base}/api/file?id=file:demo:a.rs"))
        .header("X-Comemory-Token", token)
        .header("If-Match", &new_oid)
        .body(big.clone())
        .send()
        .expect("big put");
    assert_eq!(
        big_put.status().as_u16(),
        200,
        "3 MiB save must succeed, not 413"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("a.rs"))
            .expect("read big")
            .len(),
        big.len(),
        "3 MiB content must land on disk in full"
    );

    // A stale If-Match (the now-superseded oid) → 409 Conflict.
    let res = client
        .put(format!("{base}/api/file?id=file:demo:a.rs"))
        .header("X-Comemory-Token", token)
        .header("If-Match", &old_oid)
        .body("should not be written\n")
        .send()
        .expect("stale put");
    assert_eq!(res.status().as_u16(), 409, "stale If-Match must be 409");
}
