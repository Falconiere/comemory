#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! End-to-end coverage of `POST /api/v1/mine`, `POST /api/v1/hooks/install`,
//! and `POST /api/v1/rebuild` (`src/serve/routes/maint/admin.rs`) against a
//! real bound server: the confirm gate (`mine` carries none per the route
//! table), `--repo` containment to an allowed root, the read-only 405, the
//! read-only-outranks-confirm ordering (AC-19), and AC-16's cross-process
//! rebuild-connection-swap assertion.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

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

/// Save one memory via the real binary under `home`'s data dir.
fn save_memory(home: &TempDir, body: &str) {
    AssertCommand::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args(["save", body, "--kind", "note"])
        .assert()
        .success();
}

#[test]
fn v1_mine_reports_without_writing_query_expansions() {
    let home = TempDir::new().expect("home");
    save_memory(&home, "background noise memory unrelated to any query");
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/mine"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({}))
        .send()
        .expect("v1 mine");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["meta"]["command"], "mine");
    assert_eq!(body["data"]["applied"], serde_json::json!(false));
}

#[test]
fn v1_mine_on_a_read_only_server_is_405() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &["--read-only"]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/mine"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({}))
        .send()
        .expect("v1 mine read-only");
    assert_eq!(res.status().as_u16(), 405);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "read_only");
}

#[test]
fn v1_hooks_install_without_confirm_is_400_confirmation_required() {
    // Containment is checked before the confirm gate (§Security), so `repo`
    // must sit inside an allowed root here — otherwise the assertion would
    // conflate the two gates and could observe a 403 instead.
    let home = TempDir::new().expect("home");
    let allowed = TempDir::new().expect("allowed dir");
    let (base, token, _guard) = spawn_serve(
        &home,
        &["--allow-path", allowed.path().to_str().expect("utf8 path")],
    );
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/hooks/install"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({ "repo": allowed.path().to_str().expect("utf8 path") }))
        .send()
        .expect("v1 hooks/install no confirm");
    assert_eq!(res.status().as_u16(), 400);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "confirmation_required");
}

#[test]
fn v1_hooks_install_outside_every_allowed_root_is_403_forbidden() {
    let home = TempDir::new().expect("home");
    let outside = TempDir::new().expect("outside dir");
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/hooks/install"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({
            "repo": outside.path().to_str().expect("utf8 path"),
            "confirm": true,
        }))
        .send()
        .expect("v1 hooks/install outside root");
    assert_eq!(res.status().as_u16(), 403);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "forbidden");
}

#[test]
fn v1_hooks_install_confirmed_inside_an_allow_path_root_installs_hooks() {
    // A disposable `--allow-path` root — never the checkout this test runs
    // from — so the assertion covers containment without mutating anything
    // outside the tempdir.
    let home = TempDir::new().expect("home");
    let allowed = TempDir::new().expect("allowed dir");
    let repo = allowed.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).expect("fake .git dir");
    let (base, token, _guard) = spawn_serve(
        &home,
        &["--allow-path", allowed.path().to_str().expect("utf8 path")],
    );
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/hooks/install"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({
            "repo": repo.to_str().expect("utf8 path"),
            "confirm": true,
        }))
        .send()
        .expect("v1 hooks/install confirmed");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["meta"]["command"], "hooks.install");
    assert_eq!(body["data"]["installed"].as_array().map(Vec::len), Some(3));
    for hook in ["post-commit", "post-merge", "post-checkout"] {
        assert!(repo.join(".git").join("hooks").join(hook).exists());
    }
}

/// TOCTOU regression: `req.repo` is overwritten with the SAME canonicalized
/// path `contain_abs` just verified, before `api::install_hooks::run` sees
/// it — not re-resolved from the raw (symlink) string. `Response.repo`
/// echoes `req.repo` verbatim (no internal re-canonicalization in
/// `api::install_hooks::run`), so it directly discriminates: had the
/// handler passed the raw symlink path through, `data.repo` would equal the
/// symlink path, not its resolved target.
#[test]
fn v1_hooks_install_through_a_symlink_operates_on_the_resolved_target() {
    let home = TempDir::new().expect("home");
    let allowed = TempDir::new().expect("allowed dir");
    let real_repo = allowed.path().join("real-repo");
    std::fs::create_dir_all(real_repo.join(".git")).expect("fake .git dir");
    let link = allowed.path().join("link-to-repo");
    std::os::unix::fs::symlink(&real_repo, &link).expect("create symlink");
    let canonical_real_repo = real_repo.canonicalize().expect("canonicalize real repo");
    let (base, token, _guard) = spawn_serve(
        &home,
        &["--allow-path", allowed.path().to_str().expect("utf8 path")],
    );
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/hooks/install"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({
            "repo": link.to_str().expect("utf8 path"),
            "confirm": true,
        }))
        .send()
        .expect("v1 hooks/install via symlink");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(
        body["data"]["repo"],
        serde_json::json!(canonical_real_repo.to_str().expect("utf8 path")),
        "the resolved (canonicalized) target must reach api::install_hooks::run, \
         not the raw symlink path: {body}"
    );
    for hook in ["post-commit", "post-merge", "post-checkout"] {
        assert!(
            real_repo.join(".git").join("hooks").join(hook).exists(),
            "hooks must land in the symlink's resolved target"
        );
    }
}

// ── POST /api/v1/rebuild ────────────────────────────────────────────────

/// `GET /jobs`'s reported total — used to prove a rejected `POST /rebuild`
/// created no job at all.
fn jobs_total(client: &reqwest::blocking::Client, base: &str, token: &str) -> u64 {
    let res = client
        .get(format!("{base}/api/v1/jobs"))
        .header("X-Comemory-Token", token)
        .send()
        .expect("jobs list");
    let body: serde_json::Value = res.json().expect("json");
    body["data"]["total"].as_u64().expect("total")
}

/// Poll `GET /jobs/{id}` until it reports a terminal status, returning the
/// envelope's `data` object.
fn poll_job_terminal(
    client: &reqwest::blocking::Client,
    base: &str,
    token: &str,
    job_id: &str,
) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let res = client
            .get(format!("{base}/api/v1/jobs/{job_id}"))
            .header("X-Comemory-Token", token)
            .send()
            .expect("poll job");
        let body: serde_json::Value = res.json().expect("json");
        let data = body["data"].clone();
        if matches!(data["status"].as_str(), Some("done" | "error")) {
            return data;
        }
        assert!(
            Instant::now() < deadline,
            "job {job_id} never reached a terminal status"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// `POST /api/v1/rebuild` with `body`, returning the raw response.
fn post_rebuild(
    client: &reqwest::blocking::Client,
    base: &str,
    token: &str,
    body: &serde_json::Value,
) -> reqwest::blocking::Response {
    client
        .post(format!("{base}/api/v1/rebuild"))
        .header("X-Comemory-Token", token)
        .json(body)
        .send()
        .expect("post rebuild")
}

#[test]
fn v1_rebuild_without_confirm_is_400_and_creates_no_job() {
    let home = TempDir::new().expect("home");
    save_memory(&home, "a memory that must survive nothing at all");
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();
    let before = jobs_total(&client, &base, &token);

    let res = post_rebuild(&client, &base, &token, &serde_json::json!({}));
    assert_eq!(res.status().as_u16(), 400);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "confirmation_required");
    assert_eq!(
        jobs_total(&client, &base, &token),
        before,
        "a rejected rebuild must not queue a job"
    );
}

/// AC-19: on a read-only server the read-only gate outranks the confirm
/// gate — a `POST /rebuild` **without** confirm is `405 read_only`, not
/// `400 confirmation_required`.
#[test]
fn v1_rebuild_read_only_without_confirm_is_405_not_400_ac19() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &["--read-only"]);
    let client = reqwest::blocking::Client::new();

    let res = post_rebuild(&client, &base, &token, &serde_json::json!({}));
    assert_eq!(res.status().as_u16(), 405);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "read_only");
}

#[test]
fn v1_rebuild_read_only_with_confirm_is_405_read_only() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &["--read-only"]);
    let client = reqwest::blocking::Client::new();
    let before = jobs_total(&client, &base, &token);

    let res = post_rebuild(
        &client,
        &base,
        &token,
        &serde_json::json!({ "confirm": true }),
    );
    assert_eq!(res.status().as_u16(), 405);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "read_only");
    assert_eq!(jobs_total(&client, &base, &token), before);
}

/// `POST /api/v1/memories` over HTTP, retrying while the response is a
/// `503 busy`: the rebuild job's write permit is released a hair *after* its
/// terminal status is published, so a save fired the instant the poll sees
/// `done` can legitimately lose that race. Returns the saved memory's id.
fn save_over_http(client: &reqwest::blocking::Client, base: &str, token: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let res = client
            .post(format!("{base}/api/v1/memories"))
            .header("X-Comemory-Token", token)
            .json(&serde_json::json!({
                "body": "post-rebuild memory saved over http",
                "kind": "note",
            }))
            .send()
            .expect("post memory");
        if res.status().as_u16() == 503 {
            assert!(Instant::now() < deadline, "save stayed busy for 10s");
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        let status = res.status().as_u16();
        let body: serde_json::Value = res.json().expect("json");
        assert_eq!(status, 200, "save must succeed, body: {body}");
        return body["data"]["id"].as_str().expect("saved id").to_string();
    }
}

/// Every memory id `comemory list --json` reports from a **fresh** CLI
/// process against `home`'s data dir — a brand-new process opening
/// `comemory.db` by path, so it always reads whatever inode the live path
/// points at now.
fn list_ids_from_a_fresh_cli_process(home: &TempDir) -> Vec<String> {
    let assertion = AssertCommand::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .args(["--json", "list"])
        .assert()
        .success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let page: serde_json::Value = serde_json::from_str(stdout.trim()).expect("list json");
    page["items"]
        .as_array()
        .expect("items array")
        .iter()
        .map(|i| i["id"].as_str().expect("id").to_string())
        .collect()
}

/// AC-16 — the discriminating rebuild-connection-swap assertion.
///
/// `rebuild` renames a freshly built DB over `comemory.db`, unlinking the
/// inode the server's long-lived shared connection still holds open. Without
/// `AppState::swap_conn`, the HTTP save below would land in that dead inode:
/// the save itself would still report success and a same-server search would
/// still find it, so only a **cross-process** read discriminates. The fresh
/// `comemory list --json` process opens the live path by name and therefore
/// sees the post-rebuild inode — it can only report the new id if the server
/// swapped its connection.
#[test]
fn v1_rebuild_swaps_the_shared_connection_so_a_fresh_cli_sees_later_saves_ac16() {
    let home = TempDir::new().expect("home");
    save_memory(&home, "pre-rebuild memory one about postgres pooling");
    save_memory(&home, "pre-rebuild memory two about axum routing");
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let post = post_rebuild(
        &client,
        &base,
        &token,
        &serde_json::json!({ "confirm": true }),
    );
    assert_eq!(post.status().as_u16(), 202);
    assert!(
        post.headers().get("location").is_some(),
        "202 must carry a Location header"
    );
    let post_body: serde_json::Value = post.json().expect("json");
    assert_eq!(post_body["data"]["status"], "queued");
    let job_id = post_body["data"]["job_id"]
        .as_str()
        .expect("job_id")
        .to_string();

    let job = poll_job_terminal(&client, &base, &token, &job_id);
    assert_eq!(job["status"], "done", "job body: {job}");
    assert!(
        job["result"].is_null(),
        "rebuild emits nothing on success, got {job}"
    );

    // Save AFTER the rebuild, through the same still-running server — this
    // is the write that lands in the dead inode when the swap is missing.
    let new_id = save_over_http(&client, &base, &token);

    let ids = list_ids_from_a_fresh_cli_process(&home);
    assert!(
        ids.contains(&new_id),
        "a fresh CLI process must see the post-rebuild save {new_id}; \
         got {ids:?} — the server is still writing to the unlinked pre-rebuild inode"
    );
    assert_eq!(ids.len(), 3, "two pre-rebuild memories plus the new one");
}
