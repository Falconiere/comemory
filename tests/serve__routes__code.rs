//! End-to-end coverage of `GET|POST /api/v1/code/search`
//! (`src/serve/routes/code.rs`) against a real bound server, seeded via a
//! real `comemory index-code` run over a temp git fixture repo.
//!
//! Covers GET vs POST parity and a wrong-dimension code vector (code
//! vectors are 768-dim) returning `422 vec_dim_mismatch`.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use assert_cmd::Command as AssertCommand;
use assert_cmd::cargo::cargo_bin;
use tempfile::TempDir;

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;
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

/// Index a one-file fixture repo into `home`'s data dir via the real
/// binary, then return the fixture repo's workspace tempdir (kept alive so
/// its path stays valid for the duration of the test).
fn seeded_home() -> (TempDir, TempDir) {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let repo = workspace.path().join("code-repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(
        &repo,
        &[(
            "alpha.rs",
            "fn alpha_router() {}\nfn unrelated_helper() {}\n",
        )],
        "init",
    );
    AssertCommand::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args(["index-code", "--repo", "r", "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();
    (home, workspace)
}

/// Spawn `comemory serve` on an ephemeral port, returning the base URL, the
/// session token, and the kill-on-drop guard.
fn spawn_serve(home: &TempDir) -> (String, String, ServerGuard) {
    spawn_serve_ext(home, &[])
}

/// Same as [`spawn_serve`], with extra CLI args appended after
/// `serve --port 0` (e.g. `&["--allow-path", dir]`).
fn spawn_serve_ext(home: &TempDir, extra_args: &[&str]) -> (String, String, ServerGuard) {
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
fn v1_code_search_get_and_post_agree_on_hit_ids() {
    let (home, _workspace) = seeded_home();
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let get_res = client
        .get(format!("{base}/api/v1/code/search?query=alpha_router"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 code search get");
    assert_eq!(get_res.status().as_u16(), 200);
    let get_body: serde_json::Value = get_res.json().expect("get json");
    assert_eq!(get_body["ok"], serde_json::json!(true));
    assert_eq!(get_body["meta"]["command"], "code.search");
    let get_ids: Vec<i64> = get_body["data"]["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .map(|h| h["symbol_id"].as_i64().expect("symbol_id"))
        .collect();
    assert!(!get_ids.is_empty(), "body: {get_body}");

    let post_res = client
        .post(format!("{base}/api/v1/code/search"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({ "query": "alpha_router" }))
        .send()
        .expect("v1 code search post");
    assert_eq!(post_res.status().as_u16(), 200);
    let post_body: serde_json::Value = post_res.json().expect("post json");
    let post_ids: Vec<i64> = post_body["data"]["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .map(|h| h["symbol_id"].as_i64().expect("symbol_id"))
        .collect();
    assert_eq!(get_ids, post_ids, "GET and POST must agree on hit order");
}

#[test]
fn v1_code_search_post_wrong_dim_vector_is_422_vec_dim_mismatch() {
    let (home, _workspace) = seeded_home();
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/code/search"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({ "query": "alpha_router", "vector": [0.1, 0.2, 0.3] }))
        .send()
        .expect("v1 code search wrong-dim vector");
    assert_eq!(res.status().as_u16(), 422);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(false));
    assert_eq!(body["error"]["code"], "vec_dim_mismatch");
}

#[test]
fn v1_code_search_post_well_formed_vector_succeeds() {
    let (home, _workspace) = seeded_home();
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let vector = vectors::vector("v1-code-search", 768);
    let res = client
        .post(format!("{base}/api/v1/code/search"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({ "query": "alpha_router", "vector": vector }))
        .send()
        .expect("v1 code search well-formed vector");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true), "body: {body}");
    assert!(body["data"]["hits"].is_array(), "body: {body}");
}

#[test]
fn v1_code_search_records_telemetry_like_the_cli() {
    let (home, _workspace) = seeded_home();
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/code/search?query=alpha_router"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 code search");
    let body: serde_json::Value = res.json().expect("json");
    assert!(body["data"]["query_id"].as_str().is_some(), "body: {body}");

    let db = rusqlite::Connection::open_with_flags(
        home.path().join(".comemory").join("comemory.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open db read-only");
    let source: String = db
        .query_row(
            "SELECT source FROM retrieval_log WHERE query_id = ?1",
            [body["data"]["query_id"].as_str().expect("query_id")],
            |r| r.get(0),
        )
        .expect("retrieval_log row");
    assert_eq!(source, "search-code");
}

#[test]
fn v1_code_ast_matches_inside_an_allowed_root() {
    let (home, workspace) = seeded_home();
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();
    let file = workspace.path().join("code-repo").join("alpha.rs");

    let res = client
        .post(format!("{base}/api/v1/code/ast"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({
            "pattern": "fn $NAME() {}",
            "lang": "rs",
            "file": file.to_str().expect("utf8 path"),
        }))
        .send()
        .expect("v1 code ast");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["meta"]["command"], "code.ast");
    let items = body["data"]["items"].as_array().expect("items array");
    assert_eq!(items.len(), 2, "body: {body}");
}

#[test]
fn v1_code_ast_outside_every_allowed_root_is_403_forbidden() {
    let (home, _workspace) = seeded_home();
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();
    let outside = TempDir::new().expect("outside dir");
    let file = outside.path().join("outside.rs");
    std::fs::write(&file, "fn outside() {}\n").expect("write outside file");

    let res = client
        .post(format!("{base}/api/v1/code/ast"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({
            "pattern": "fn $NAME() {}",
            "lang": "rs",
            "file": file.to_str().expect("utf8 path"),
        }))
        .send()
        .expect("v1 code ast outside root");
    assert_eq!(res.status().as_u16(), 403);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "forbidden");
}

#[test]
fn v1_code_ast_nonexistent_file_is_400_bad_request() {
    let (home, workspace) = seeded_home();
    let (base, token, _guard) = spawn_serve(&home);
    let client = reqwest::blocking::Client::new();
    let file = workspace.path().join("code-repo").join("nope.rs");

    let res = client
        .post(format!("{base}/api/v1/code/ast"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({
            "pattern": "fn $NAME() {}",
            "lang": "rs",
            "file": file.to_str().expect("utf8 path"),
        }))
        .send()
        .expect("v1 code ast nonexistent file");
    assert_eq!(res.status().as_u16(), 400);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "bad_request");
}
