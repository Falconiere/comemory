//! End-to-end coverage of `GET /api/v1/sources` (`src/serve/routes/sources.rs`)
//! against a real bound server, including the read-only reconcile gate: the
//! route computes `reconcile` from `--read-only` server-side, never from the
//! query string (mirrors `tests/api__sources.rs`, which exercises
//! `api::sources::run` directly without HTTP). `DELETE
//! /sources?target=&confirm=true` (`api::unindex`) coverage lives here too.

#[path = "common/docs_fixtures.rs"]
mod docs_fixtures;

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

/// Register one real docs fixture directory as a source in `home`; returns
/// the fixture workspace kept alive for the duration of the test plus the
/// docs directory's canonical path (a valid `unindex` target).
fn seeded_home() -> (TempDir, TempDir, std::path::PathBuf) {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());
    AssertCommand::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args([
            "index",
            docs.to_str().expect("utf8 path"),
            "--repo",
            "docs-corpus",
        ])
        .assert()
        .success();
    (home, workspace, docs)
}

/// Drop every entry from `sources.toml`, simulating an out-of-band edit that
/// leaves the mirror row registered but the durable registry empty.
fn clear_registry(home: &TempDir) {
    std::fs::write(
        home.path().join(".comemory").join("sources.toml"),
        "format = 1\n",
    )
    .expect("rewrite sources.toml");
}

#[test]
fn v1_sources_returns_the_registered_source_envelope() {
    let (home, _workspace, _docs) = seeded_home();
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/sources"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 sources");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["meta"]["command"], "sources");

    let rows = body["data"].as_array().expect("data array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["repo"], "docs-corpus");
    assert_eq!(
        rows[0]["indexed"].as_u64(),
        Some(docs_fixtures::FIXTURE_COUNT as u64)
    );
}

#[test]
fn v1_sources_read_only_server_skips_the_mirror_reconcile() {
    let (home, _workspace, _docs) = seeded_home();
    clear_registry(&home);
    let (base, token, _guard) = spawn_serve(&home, &["--read-only"]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .get(format!("{base}/api/v1/sources"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 sources read-only");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    let rows = body["data"].as_array().expect("data array");
    assert_eq!(
        rows.len(),
        1,
        "a read-only server must not reconcile away the now-unregistered row"
    );
}

#[test]
fn v1_unindex_without_confirm_is_400_confirmation_required() {
    let (home, _workspace, docs) = seeded_home();
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .delete(format!("{base}/api/v1/sources"))
        .header("X-Comemory-Token", &token)
        .query(&[("target", docs.to_str().expect("utf8 path"))])
        .send()
        .expect("v1 unindex no confirm");
    assert_eq!(res.status().as_u16(), 400);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "confirmation_required");

    // The source must survive an unconfirmed DELETE.
    let follow_up = client
        .get(format!("{base}/api/v1/sources"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 sources follow-up");
    let follow_up_body: serde_json::Value = follow_up.json().expect("json");
    assert_eq!(follow_up_body["data"].as_array().map(Vec::len), Some(1));
}

#[test]
fn v1_unindex_on_a_read_only_server_is_405() {
    let (home, _workspace, docs) = seeded_home();
    let (base, token, _guard) = spawn_serve(&home, &["--read-only"]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .delete(format!("{base}/api/v1/sources"))
        .header("X-Comemory-Token", &token)
        .query(&[
            ("target", docs.to_str().expect("utf8 path")),
            ("confirm", "true"),
        ])
        .send()
        .expect("v1 unindex read-only");
    assert_eq!(res.status().as_u16(), 405);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "read_only");
}

/// Poll `GET /jobs/{id}` until it reports a terminal status, returning the
/// envelope's `data` object.
fn poll_job_terminal(
    client: &reqwest::blocking::Client,
    base: &str,
    token: &str,
    job_id: &str,
    timeout: std::time::Duration,
) -> serde_json::Value {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let res = client
            .get(format!("{base}/api/v1/jobs/{job_id}"))
            .header("X-Comemory-Token", token)
            .send()
            .expect("poll job");
        let body: serde_json::Value = res.json().expect("json");
        let data = body["data"].clone();
        if matches!(data["status"].as_str(), Some("done") | Some("error")) {
            return data;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "job {job_id} never reached a terminal status"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// `GET /jobs`'s reported total, for the containment-rejection check.
fn jobs_total(client: &reqwest::blocking::Client, base: &str, token: &str) -> u64 {
    let res = client
        .get(format!("{base}/api/v1/jobs"))
        .header("X-Comemory-Token", token)
        .send()
        .expect("jobs list");
    let body: serde_json::Value = res.json().expect("json");
    body["data"]["total"].as_u64().expect("total")
}

/// `POST /api/v1/sources {path, repo?}`: asserts the `202` shape (status,
/// `Location` header) and returns the new job id.
fn post_sources_expect_202(
    client: &reqwest::blocking::Client,
    base: &str,
    token: &str,
    payload: &serde_json::Value,
) -> String {
    let post = client
        .post(format!("{base}/api/v1/sources"))
        .header("X-Comemory-Token", token)
        .json(payload)
        .send()
        .expect("post sources");
    assert_eq!(post.status().as_u16(), 202);
    assert!(
        post.headers().get("location").is_some(),
        "202 must carry a Location header"
    );
    let body: serde_json::Value = post.json().expect("json");
    body["data"]["job_id"].as_str().expect("job_id").to_string()
}

/// `POST /api/v1/sources` — job-creating registration, real fixtures.
#[test]
fn v1_post_sources_job_registers_and_indexes_real_fixtures() {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());
    let (base, token, _guard) = spawn_serve(
        &home,
        &["--allow-path", workspace.path().to_str().expect("utf8")],
    );
    let client = reqwest::blocking::Client::new();

    let job_id = post_sources_expect_202(
        &client,
        &base,
        &token,
        &serde_json::json!({
            "path": [docs.to_str().expect("utf8 path")],
            "repo": "docs-corpus",
        }),
    );
    let job = poll_job_terminal(
        &client,
        &base,
        &token,
        &job_id,
        std::time::Duration::from_secs(20),
    );
    assert_eq!(job["status"], "done", "job body: {job}");
    let sources = job["result"]["sources"].as_array().expect("sources array");
    assert_eq!(sources.len(), 1);
    assert_eq!(
        sources[0]["indexed"].as_u64(),
        Some(docs_fixtures::FIXTURE_COUNT as u64)
    );

    let get_res = client
        .get(format!("{base}/api/v1/sources"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("get sources after job");
    let get_body: serde_json::Value = get_res.json().expect("json");
    assert_eq!(get_body["data"].as_array().map(Vec::len), Some(1));
}

/// `POST /api/v1/sources` — a path outside every allowed root is `403` and
/// creates no job.
#[test]
fn v1_post_sources_outside_every_allowed_root_is_403_and_creates_no_job() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();
    let outside = TempDir::new().expect("outside dir");
    std::fs::write(outside.path().join("f.md"), "hi\n").expect("write file");
    let before = jobs_total(&client, &base, &token);

    let res = client
        .post(format!("{base}/api/v1/sources"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({
            "path": [outside.path().to_str().expect("utf8 path")],
        }))
        .send()
        .expect("post sources outside root");
    assert_eq!(res.status().as_u16(), 403);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "forbidden");
    assert_eq!(
        jobs_total(&client, &base, &token),
        before,
        "no job must be created"
    );
}

/// TOCTOU regression: `req.path` entries are overwritten with the SAME
/// canonicalized path `contain_abs` just verified before `api::index::run`
/// sees them (§Security "Path containment"). Registering through a symlink
/// inside the allowed root must succeed and land the source at its resolved
/// (canonical) target, not the symlink path.
#[test]
fn v1_post_sources_through_a_symlink_registers_the_resolved_target() {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());
    let canonical_docs = docs.canonicalize().expect("canonicalize docs dir");
    let link = workspace.path().join("docs-link");
    std::os::unix::fs::symlink(&docs, &link).expect("create symlink");
    let (base, token, _guard) = spawn_serve(
        &home,
        &["--allow-path", workspace.path().to_str().expect("utf8")],
    );
    let client = reqwest::blocking::Client::new();

    let job_id = post_sources_expect_202(
        &client,
        &base,
        &token,
        &serde_json::json!({
            "path": [link.to_str().expect("utf8 path")],
            "repo": "docs-corpus",
        }),
    );
    let job = poll_job_terminal(
        &client,
        &base,
        &token,
        &job_id,
        std::time::Duration::from_secs(20),
    );
    assert_eq!(job["status"], "done", "job body: {job}");
    let sources = job["result"]["sources"].as_array().expect("sources array");
    assert_eq!(sources.len(), 1);
    assert_eq!(
        sources[0]["canonical_path"],
        serde_json::json!(canonical_docs.to_str().expect("utf8 path")),
        "the registered source must resolve to the symlink's target, not the symlink itself: {job}"
    );
    assert_eq!(
        sources[0]["indexed"].as_u64(),
        Some(docs_fixtures::FIXTURE_COUNT as u64)
    );
}

#[test]
fn v1_unindex_confirmed_removes_the_registered_source() {
    let (home, _workspace, docs) = seeded_home();
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .delete(format!("{base}/api/v1/sources"))
        .header("X-Comemory-Token", &token)
        .query(&[
            ("target", docs.to_str().expect("utf8 path")),
            ("confirm", "true"),
        ])
        .send()
        .expect("v1 unindex confirmed");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["meta"]["command"], "unindex");
    assert_eq!(
        body["data"]["documents_removed"].as_u64(),
        Some(docs_fixtures::FIXTURE_COUNT as u64)
    );

    let follow_up = client
        .get(format!("{base}/api/v1/sources"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("v1 sources follow-up");
    let follow_up_body: serde_json::Value = follow_up.json().expect("json");
    assert_eq!(follow_up_body["data"].as_array().map(Vec::len), Some(0));
}
