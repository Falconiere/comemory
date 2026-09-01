#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! End-to-end coverage of the mutating memory routes
//! (`src/serve/routes/memories/write.rs`) against a real bound server:
//! `POST /api/v1/memories` (AC-1), `DELETE /api/v1/memories/{id}?confirm=`
//! (AC-5), `POST /api/v1/feedback`, and the read-only 405 sweep. Mirrors
//! the `ServerGuard`/`spawn_serve` harness in
//! `tests/serve__routes__maint__mod.rs`.

#[path = "common/vectors.rs"]
mod vectors;

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
        .filter_map(Result::ok)
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

/// A real, valid NDJSON ingest-code row body big enough to hold the write
/// permit for a measurable window — deterministic vectors via `vectors`.
fn bulk_ingest_body(rows: usize) -> String {
    let mut body = String::new();
    for i in 0..rows {
        let seed = format!("ac17-{i}");
        let embedding = vectors::vector(&seed, 768);
        let row = serde_json::json!({
            "repo": "bulk",
            "path": format!("src/f{i}.rs"),
            "blob_oid": format!("{i:040x}"),
            "symbol": format!("sym_{i}"),
            "kind": "function",
            "lang": "rust",
            "line_start": 1_u32,
            "line_end": 3_u32,
            "snippet": format!("fn sym_{i}() {{}}"),
            "simhash": 0_i64,
            "embedding": embedding,
        });
        body.push_str(&row.to_string());
        body.push('\n');
    }
    body
}

/// Poll `GET /jobs/{id}` until it reports a terminal status.
fn poll_job_done(
    client: &reqwest::blocking::Client,
    base: &str,
    token: &str,
    job_id: &str,
    timeout: std::time::Duration,
) {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let res = client
            .get(format!("{base}/api/v1/jobs/{job_id}"))
            .header("X-Comemory-Token", token)
            .send()
            .expect("poll job");
        let body: serde_json::Value = res.json().expect("json");
        // `.expect`, not `.unwrap_or_default()`: a missing `status` means a
        // malformed job response, which should fail here rather than spin
        // the loop to its deadline with an empty status.
        let status = body["data"]["status"]
            .as_str()
            .unwrap_or_else(|| panic!("job {job_id} response has no data.status: {body}"));
        if status == "done" || status == "error" {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "job {job_id} never reached a terminal status (stuck in {status})"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// AC-17: while a real `ingest-code` job holds the single write permit,
/// `POST /api/v1/memories` returns `503 busy` with a `Retry-After` header —
/// no 5s stall; once the job finishes, the same save succeeds. A large
/// real NDJSON batch gives the job a measurable duration; the assertion
/// polls a tight loop (not a single well-timed shot) so the test is not a
/// race against job scheduling.
/// Loop `POST /memories` against a still-running job, returning `true` the
/// moment a `503 busy` (with its `Retry-After` header) is observed, or
/// `false` once the job reaches a terminal status without ever producing
/// one.
fn poll_for_busy_save(
    client: &reqwest::blocking::Client,
    base: &str,
    token: &str,
    job_id: &str,
    save_payload: &serde_json::Value,
) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        let res = client
            .post(format!("{base}/api/v1/memories"))
            .header("X-Comemory-Token", token)
            .json(save_payload)
            .send()
            .expect("post memories during job");
        if res.status().as_u16() == 503 {
            assert!(
                res.headers().get("retry-after").is_some(),
                "503 busy must carry a Retry-After header"
            );
            let body: serde_json::Value = res.json().expect("json");
            assert_eq!(body["error"]["code"], "busy");
            return true;
        }
        let status_res = client
            .get(format!("{base}/api/v1/jobs/{job_id}"))
            .header("X-Comemory-Token", token)
            .send()
            .expect("poll job status");
        let status_body: serde_json::Value = status_res.json().expect("json");
        if matches!(
            status_body["data"]["status"].as_str(),
            Some("done" | "error")
        ) {
            return false;
        }
    }
    false
}

#[test]
fn v1_post_memories_returns_503_busy_while_a_job_holds_the_write_permit_ac17() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let ingest_res = client
        .post(format!("{base}/api/v1/code/ingest"))
        .header("X-Comemory-Token", &token)
        .body(bulk_ingest_body(4000))
        .send()
        .expect("post ingest");
    assert_eq!(ingest_res.status().as_u16(), 202);
    let ingest_body: serde_json::Value = ingest_res.json().expect("json");
    let job_id = ingest_body["data"]["job_id"]
        .as_str()
        .expect("job_id")
        .to_string();

    let save_payload = serde_json::json!({
        "body": "a memory saved over http while an ingest-code job holds the write permit",
    });
    let saw_busy = poll_for_busy_save(&client, &base, &token, &job_id, &save_payload);
    assert!(
        saw_busy,
        "must observe 503 busy at least once while the ingest job holds the write permit"
    );

    poll_job_done(
        &client,
        &base,
        &token,
        &job_id,
        std::time::Duration::from_secs(20),
    );
    let final_res = client
        .post(format!("{base}/api/v1/memories"))
        .header("X-Comemory-Token", &token)
        .json(&save_payload)
        .send()
        .expect("post memories after job finished");
    assert_eq!(final_res.status().as_u16(), 200);
}

/// AC-18 against the real bound server: a `POST /api/v1/memories` body over
/// the global 5 MiB limit is refused by axum's own `DefaultBodyLimit`
/// before the handler runs, and nothing is stored.
///
/// Over a socket the rejection races the upload — the server answers and
/// closes as soon as the limit is crossed, so a loaded machine can surface
/// a write reset to the client instead of the response. Both endings prove
/// the same refusal, so both are accepted here, and the exact `413` is
/// pinned in-process instead (`src/serve/tests/router.rs`), where the whole
/// request is delivered at once and no race exists. What this test adds is
/// what only a real server can show: it is still serving afterwards, and
/// the oversized memory does not exist.
#[test]
fn v1_post_memories_over_5mib_is_refused_and_stores_nothing_ac18() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &[]);
    // Connection pooling OFF for this test only. The server closes the
    // connection as soon as it refuses the oversized body, and a pooled
    // client hands that same half-closed connection to the follow-up
    // request, which then fails with `IncompleteMessage` — a client-side
    // artifact of the refusal, not a server that stopped serving. A fresh
    // connection per request keeps the survival check honest.
    let client = reqwest::blocking::Client::builder()
        .pool_max_idle_per_host(0)
        .build()
        .expect("build client");

    let oversized = "a".repeat(6 * 1024 * 1024);
    let payload = serde_json::json!({ "body": oversized });
    let result = client
        .post(format!("{base}/api/v1/memories"))
        .header("X-Comemory-Token", &token)
        .json(&payload)
        .send();
    match result {
        Ok(res) => assert_eq!(res.status().as_u16(), 413, "body: {:?}", res.text()),
        Err(e) => assert!(
            e.is_request(),
            "the only tolerated failure is the server closing the connection \
             mid-upload after refusing it: {e:?}"
        ),
    }

    let listed = client
        .get(format!("{base}/api/v1/memories"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("the server survives an oversized request");
    assert_eq!(listed.status().as_u16(), 200);
    let body: serde_json::Value = listed.json().expect("memories list json");
    assert_eq!(
        body["data"]["items"].as_array().map(Vec::len),
        Some(0),
        "nothing over the limit was stored: {body}"
    );
}
