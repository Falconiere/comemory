//! End-to-end coverage of the job-creating code routes
//! (`src/serve/routes/code.rs`): `POST /api/v1/code/index` (AC-6, AC-7,
//! AC-20) and `POST /api/v1/code/ingest` (happy path, AC-18). Split out of
//! `tests/serve__routes__code.rs` to stay under the file-size ceiling.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

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
/// binary, then return the fixture repo's workspace tempdir. The stamped
/// `repo_marker.root_path` (from `index-code`) becomes the one allowed
/// root for tests that need containment to succeed or fail predictably.
fn seeded_home() -> (TempDir, TempDir) {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let repo = workspace.path().join("code-repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(&repo, &[("alpha.rs", "fn alpha_router() {}\n")], "init");
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
/// session token, and the kill-on-drop guard. `extra_args` is appended
/// after `serve --port 0` (e.g. `&["--allow-path", dir]`). Access tracking
/// is disabled (AC-6: "`GET /memories/search` (tracking off)") so a
/// concurrent search's `retrieval_log`/`access_count` write never contends
/// at the SQLite level with a job's own connection holding a write
/// transaction — the two are separate connections, and only mutating
/// routes take the in-process write permit; a tracked search's write is a
/// side effect the permit was never meant to serialize against a job.
fn spawn_serve(home: &TempDir, extra_args: &[&str]) -> (String, String, ServerGuard) {
    let mut child = Command::new(cargo_bin("comemory"))
        .env("COMEMORY_DATA_DIR", home.path().join(".comemory"))
        .env("COMEMORY_DISABLE_ACCESS_TRACKING", "true")
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

/// Poll `GET /jobs/{id}` until it reports a terminal status, returning the
/// envelope's `data` object.
fn poll_job_terminal(
    client: &reqwest::blocking::Client,
    base: &str,
    token: &str,
    job_id: &str,
    timeout: Duration,
) -> serde_json::Value {
    let deadline = Instant::now() + timeout;
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
            Instant::now() < deadline,
            "job {job_id} never reached a terminal status"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// `GET /jobs`'s reported total, for the AC-7 "no job was created" checks.
fn jobs_total(client: &reqwest::blocking::Client, base: &str, token: &str) -> u64 {
    let res = client
        .get(format!("{base}/api/v1/jobs"))
        .header("X-Comemory-Token", token)
        .send()
        .expect("jobs list");
    let body: serde_json::Value = res.json().expect("json");
    body["data"]["total"].as_u64().expect("total")
}

/// `POST /api/v1/code/index {repo, path}`, returning the raw response.
fn post_code_index(
    client: &reqwest::blocking::Client,
    base: &str,
    token: &str,
    repo: &str,
    path: &std::path::Path,
) -> reqwest::blocking::Response {
    client
        .post(format!("{base}/api/v1/code/index"))
        .header("X-Comemory-Token", token)
        .json(&serde_json::json!({
            "repo": repo,
            "path": path.to_str().expect("utf8 path"),
        }))
        .send()
        .expect("post code/index")
}

/// Build a real, not-yet-indexed fixture repo with `n` trivial Rust files.
fn build_fresh_repo(root: &std::path::Path, n: usize) -> std::path::PathBuf {
    let repo = root.join("fresh-repo");
    git_repo::init_repo(&repo);
    let files: Vec<(String, String)> = (0..n)
        .map(|i| (format!("f{i}.rs"), format!("fn f{i}_fn() {{}}\n")))
        .collect();
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, c)| (p.as_str(), c.as_str()))
        .collect();
    git_commit::commit_files(&repo, &refs, "init");
    repo
}

/// AC-6: `POST /code/index` on a real, not-yet-indexed fixture repo reaches
/// `status:"done"` with `code_symbols` rows present.
#[test]
fn v1_code_index_job_runs_to_done_and_writes_symbols_ac6() {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let repo = build_fresh_repo(workspace.path(), 3);
    let (base, token, _guard) = spawn_serve(
        &home,
        &["--allow-path", workspace.path().to_str().expect("utf8")],
    );
    let client = reqwest::blocking::Client::new();

    let post = post_code_index(&client, &base, &token, "fresh", &repo);
    assert_eq!(post.status().as_u16(), 202);
    assert!(
        post.headers().get("location").is_some(),
        "202 must carry a Location header"
    );
    let post_body: serde_json::Value = post.json().expect("json");
    assert_eq!(post_body["ok"], serde_json::json!(true));
    assert_eq!(post_body["data"]["status"], "queued");
    let job_id = post_body["data"]["job_id"]
        .as_str()
        .expect("job_id")
        .to_string();

    // Concurrent read: issued right after the 202, not blocked by the job.
    let started = Instant::now();
    let search_res = client
        .get(format!("{base}/api/v1/memories/search?query=alpha"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("concurrent memories search");
    let elapsed = started.elapsed();
    let search_status = search_res.status().as_u16();
    let search_body_text = search_res.text().unwrap_or_default();
    assert_eq!(
        search_status, 200,
        "search must succeed, body: {search_body_text}"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "search must answer in <1s while the job runs, took {elapsed:?}"
    );

    let job = poll_job_terminal(&client, &base, &token, &job_id, Duration::from_secs(20));
    assert_eq!(job["status"], "done", "job body: {job}");
    assert_eq!(job["result"]["repo"], "fresh");
    assert_eq!(job["result"]["files_indexed"].as_u64(), Some(3));
    assert_symbols_at_least(&home, "fresh", 3);
}

/// Assert `code_symbols` carries at least `min` rows for `repo`.
fn assert_symbols_at_least(home: &TempDir, repo: &str, min: i64) {
    let db = rusqlite::Connection::open_with_flags(
        home.path().join(".comemory").join("comemory.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open db read-only");
    let symbols: i64 = db
        .query_row(
            "SELECT count(*) FROM code_symbols WHERE repo = ?1",
            [repo],
            |r| r.get(0),
        )
        .expect("count code_symbols");
    assert!(symbols >= min, "symbols: {symbols}");
}

/// TOCTOU regression: `req.file` is overwritten with the SAME canonicalized
/// path `contain_abs` just verified before `api::ast::run` reads it
/// (§Security "Path containment") — a file reached only through a symlink
/// inside the allowed root must still be found and searched.
#[test]
fn v1_code_ast_through_a_symlink_inside_the_allowed_root_finds_matches() {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let real_file = workspace.path().join("real.rs");
    std::fs::write(&real_file, "fn alpha_router() {}\n").expect("write file");
    let link = workspace.path().join("link.rs");
    std::os::unix::fs::symlink(&real_file, &link).expect("create symlink");
    let (base, token, _guard) = spawn_serve(
        &home,
        &["--allow-path", workspace.path().to_str().expect("utf8")],
    );
    let client = reqwest::blocking::Client::new();

    let res = client
        .post(format!("{base}/api/v1/code/ast"))
        .header("X-Comemory-Token", &token)
        .json(&serde_json::json!({
            "pattern": "fn $NAME() {}",
            "lang": "rust",
            "file": link.to_str().expect("utf8 path"),
        }))
        .send()
        .expect("post code/ast via symlink");
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().expect("json");
    let items = body["data"]["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1, "body: {body}");
}

/// AC-7: a path outside every allowed root is `403` and creates no job.
#[test]
fn v1_code_index_outside_every_allowed_root_is_403_and_creates_no_job_ac7() {
    let (home, _workspace) = seeded_home();
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();
    let before = jobs_total(&client, &base, &token);
    let outside = TempDir::new().expect("outside dir");

    let res = post_code_index(&client, &base, &token, "outside", outside.path());
    assert_eq!(res.status().as_u16(), 403);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "forbidden");
    assert_eq!(
        jobs_total(&client, &base, &token),
        before,
        "no job must be created"
    );
}

/// AC-7: a `..`-laden path resolving outside every allowed root is also
/// `403`, no job created.
#[test]
fn v1_code_index_dot_dot_laden_path_is_403_ac7() {
    let (home, workspace) = seeded_home();
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();
    let before = jobs_total(&client, &base, &token);
    let escape = workspace.path().join("code-repo").join("..").join("..");

    let res = post_code_index(&client, &base, &token, "escape", &escape);
    assert_eq!(res.status().as_u16(), 403);
    assert_eq!(jobs_total(&client, &base, &token), before);
}

/// AC-7: a symlink inside an allowed root pointing outside it is `403`
/// once canonicalized, no job created.
#[test]
fn v1_code_index_symlink_escaping_path_is_403_ac7() {
    let (home, workspace) = seeded_home();
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();
    let before = jobs_total(&client, &base, &token);

    let secret = TempDir::new().expect("secret dir");
    let link = workspace.path().join("code-repo").join("escape-link");
    std::os::unix::fs::symlink(secret.path(), &link).expect("create symlink");

    let res = post_code_index(&client, &base, &token, "escape", &link);
    assert_eq!(res.status().as_u16(), 403);
    assert_eq!(jobs_total(&client, &base, &token), before);
}

/// TOCTOU regression: `req.path` is overwritten with the SAME canonicalized
/// path `contain_abs` just verified before the job body walks it (§Security
/// "Path containment") — indexing through a symlink inside the allowed root
/// must succeed and record the resolved (canonical) root, not the symlink
/// path.
#[test]
fn v1_code_index_through_a_symlink_records_the_resolved_root() {
    let (home, workspace) = seeded_home();
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let real_repo = workspace.path().join("code-repo");
    let canonical_repo = real_repo.canonicalize().expect("canonicalize code-repo");
    let link = workspace.path().join("repo-link");
    std::os::unix::fs::symlink(&real_repo, &link).expect("create symlink");

    let post = post_code_index(&client, &base, &token, "linked", &link);
    assert_eq!(post.status().as_u16(), 202);
    let post_body: serde_json::Value = post.json().expect("json");
    let job_id = post_body["data"]["job_id"]
        .as_str()
        .expect("job_id")
        .to_string();
    let job = poll_job_terminal(&client, &base, &token, &job_id, Duration::from_secs(20));
    assert_eq!(job["status"], "done", "job body: {job}");

    let stored_root = repo_marker_root_path(&home, "linked");
    assert_eq!(
        stored_root.as_deref(),
        Some(canonical_repo.to_str().expect("utf8 path")),
        "repo_marker.root_path must be the symlink's resolved target, not the symlink itself"
    );
}

/// `repo_marker.root_path` for `repo`, or `None` when absent/NULL.
fn repo_marker_root_path(home: &TempDir, repo: &str) -> Option<String> {
    let db = rusqlite::Connection::open_with_flags(
        home.path().join(".comemory").join("comemory.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open db read-only");
    db.query_row(
        "SELECT root_path FROM repo_marker WHERE repo = ?1",
        [repo],
        |r| r.get(0),
    )
    .expect("query repo_marker")
}

/// AC-7: a nonexistent path is `400`, no job created.
#[test]
fn v1_code_index_nonexistent_path_is_400_ac7() {
    let (home, workspace) = seeded_home();
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();
    let before = jobs_total(&client, &base, &token);
    let missing = workspace.path().join("code-repo").join("does-not-exist");

    let res = post_code_index(&client, &base, &token, "missing", &missing);
    assert_eq!(res.status().as_u16(), 400);
    let body: serde_json::Value = res.json().expect("json");
    assert_eq!(body["error"]["code"], "bad_request");
    assert_eq!(jobs_total(&client, &base, &token), before);
}

/// AC-20: a valid in-root path that is NOT a git work tree drives the job
/// to `status:"error"` (`result` null, an enveloped error), and the SSE
/// stream ends after an `error` event.
#[test]
fn v1_code_index_non_git_directory_job_errors_ac20() {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let not_git = workspace.path().join("not-a-repo");
    std::fs::create_dir_all(&not_git).expect("mkdir");
    std::fs::write(not_git.join("a.rs"), "fn a() {}\n").expect("write file");
    let (base, token, _guard) = spawn_serve(
        &home,
        &["--allow-path", workspace.path().to_str().expect("utf8")],
    );
    let client = reqwest::blocking::Client::new();

    let post = post_code_index(&client, &base, &token, "notgit", &not_git);
    assert_eq!(post.status().as_u16(), 202);
    let post_body: serde_json::Value = post.json().expect("json");
    let job_id = post_body["data"]["job_id"]
        .as_str()
        .expect("job_id")
        .to_string();

    let job = poll_job_terminal(&client, &base, &token, &job_id, Duration::from_secs(20));
    assert_eq!(job["status"], "error", "job body: {job}");
    assert!(job["result"].is_null());
    assert!(job["error"]["code"].as_str().is_some(), "job body: {job}");
    assert_sse_ends_with_error(&client, &base, &token, &job_id);
}

/// A finished job's SSE stream ends after emitting the error event — the
/// plain blocking `.text()` read completes once the connection closes,
/// which the handler does itself once the terminal event lands.
fn assert_sse_ends_with_error(
    client: &reqwest::blocking::Client,
    base: &str,
    token: &str,
    job_id: &str,
) {
    let sse_text = client
        .get(format!("{base}/api/v1/jobs/{job_id}/events"))
        .header("X-Comemory-Token", token)
        .send()
        .expect("sse request")
        .text()
        .expect("sse body");
    assert!(
        sse_text.contains("event: error"),
        "sse body must carry an error event: {sse_text}"
    );
}

/// Happy-path `POST /code/ingest` job: real NDJSON row, done, real rows.
#[test]
fn v1_code_ingest_job_runs_to_done_and_writes_rows() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let row = serde_json::json!({
        "repo": "ingest-sample",
        "path": "src/a.rs",
        "blob_oid": "aaaa000000000000000000000000000000000000",
        "symbol": "sym_a",
        "kind": "function",
        "lang": "rust",
        "line_start": 1_u32,
        "line_end": 3_u32,
        "snippet": "fn sym_a() {}",
        "simhash": 0_i64,
        "embedding": vectors::vector("ingest-sample", 768),
    });

    let post = client
        .post(format!("{base}/api/v1/code/ingest"))
        .header("X-Comemory-Token", &token)
        .body(format!("{row}\n"))
        .send()
        .expect("post code/ingest");
    assert_eq!(post.status().as_u16(), 202);
    let post_body: serde_json::Value = post.json().expect("json");
    let job_id = post_body["data"]["job_id"]
        .as_str()
        .expect("job_id")
        .to_string();

    let job = poll_job_terminal(&client, &base, &token, &job_id, Duration::from_secs(20));
    assert_eq!(job["status"], "done", "job body: {job}");
    assert_eq!(job["result"]["files"].as_u64(), Some(1));
    assert_eq!(job["result"]["rows"].as_u64(), Some(1));
    assert_symbols_at_least(&home, "ingest-sample", 1);
}

/// AC-18: a 6 MiB `POST /code/ingest` body clears the per-route 64 MiB
/// limit (content is arbitrary — this asserts acceptance, not success).
#[test]
fn v1_code_ingest_accepts_a_6mib_body_ac18() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let body = "x".repeat(6 * 1024 * 1024);
    let res = client
        .post(format!("{base}/api/v1/code/ingest"))
        .header("X-Comemory-Token", &token)
        .body(body)
        .send()
        .expect("post 6mib ingest");
    assert_eq!(
        res.status().as_u16(),
        202,
        "6 MiB must clear the per-route 64 MiB limit and still create a job"
    );
}

/// AC-18: a body over the per-route 64 MiB limit is a plain framework
/// `413`, not the JSON envelope.
#[test]
fn v1_code_ingest_rejects_a_body_over_64mib_ac18() {
    let home = TempDir::new().expect("home");
    let (base, token, _guard) = spawn_serve(&home, &[]);
    let client = reqwest::blocking::Client::new();

    let body = "x".repeat(64 * 1024 * 1024 + 1024);
    let res = client
        .post(format!("{base}/api/v1/code/ingest"))
        .header("X-Comemory-Token", &token)
        .body(body)
        .send()
        .expect("post over 64mib ingest");
    assert_eq!(res.status().as_u16(), 413);
}
