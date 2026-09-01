#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! In-process coverage of `src/serve/routes/index_runs.rs`:
//! `POST /api/v1/index/runs` (its gate order, the `root`/`paths` aliases,
//! the two index modes) and `GET /api/v1/index/runs` (the history the jobs
//! write). Real temp git repos, the real router, real background jobs.
//!
//! AC-10 is proved deterministically rather than by racing: the test holds
//! the server's single write permit, which pins the first job in `queued`
//! (a mutating job awaits that permit before running), so the second
//! `POST` is guaranteed to see it live.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use comemory::config::{Config, Paths};
use comemory::serve::{AppState, RootOverrides, ServeOptions};
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::test_common::git_sample;
use crate::test_common::serve_state::{self, Session};

/// One in-process session plus the `AppState` behind it (the fixture's own
/// `session_with` keeps that private, and these tests need the write permit
/// and the shared connection).
struct Fixture {
    session: Session,
    state: AppState,
    /// Kept alive: dropping it would delete the indexed repo.
    _workspace: TempDir,
    repo: PathBuf,
}

fn fixture(read_only: bool) -> Fixture {
    let workspace = TempDir::new().expect("workspace");
    let repo = git_sample::build_sample_repo(workspace.path());
    let home = TempDir::new().expect("home");
    let paths = Paths::new(home.path());
    let mut roots = RootOverrides::new();
    roots.insert("sample".to_string(), repo.clone());
    let opts = ServeOptions {
        repo: None,
        port: 0,
        read_only,
        roots,
        cfg: Config::defaults(),
        embed_cmd: None,
        allow_path: Vec::new(),
    };
    let state = AppState::new(&paths, opts).expect("AppState::new");
    let token = state.token().to_string();
    let router = comemory::serve::router::build_router(state.clone());
    Fixture {
        session: Session {
            home,
            token,
            router,
        },
        state,
        _workspace: workspace,
        repo,
    }
}

fn repo_path(f: &Fixture) -> String {
    f.repo.to_str().expect("utf8 path").to_string()
}

async fn post_run(f: &Fixture, body: Value) -> serve_state::Resp {
    serve_state::send(&f.session, "POST", "/api/v1/index/runs", Some(body)).await
}

/// Poll `GET /jobs/{id}` until terminal, returning the job view.
async fn poll_job(f: &Fixture, job_id: &str) -> Value {
    for _ in 0..1_000 {
        let resp =
            serve_state::send(&f.session, "GET", &format!("/api/v1/jobs/{job_id}"), None).await;
        let data = resp.json["data"].clone();
        if matches!(
            data["status"].as_str(),
            Some("done" | "error" | "cancelled")
        ) {
            return data;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("job {job_id} never reached a terminal status");
}

/// Start one run and wait for it, returning the finished job view.
async fn run_to_completion(f: &Fixture, body: Value) -> Value {
    let accepted = post_run(f, body).await;
    assert_eq!(accepted.status, 202, "body: {}", accepted.text);
    let job_id = accepted.json["data"]["job_id"]
        .as_str()
        .expect("job_id")
        .to_string();
    let job = poll_job(f, &job_id).await;
    assert_eq!(job["status"], "done", "job: {job}");
    job
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_run_for_one_repo_is_409_index_running_and_the_first_still_finishes() {
    let f = fixture(false);
    let path = repo_path(&f);
    // Pinning the write permit keeps the first job `queued` — a mutating
    // job awaits it before running — so the conflict is deterministic.
    let permit = Arc::clone(f.state.write_permit())
        .acquire_owned()
        .await
        .expect("write permit");

    let first = post_run(&f, json!({"repo": "sample", "path": path})).await;
    assert_eq!(first.status, 202, "body: {}", first.text);
    let job_id = first.json["data"]["job_id"]
        .as_str()
        .expect("job_id")
        .to_string();

    let second = post_run(&f, json!({"repo": "sample", "path": path})).await;
    assert_eq!(second.status, 409, "body: {}", second.text);
    assert_eq!(second.json["error"]["code"], "index_running");
    assert_eq!(second.json["error"]["details"]["job_id"], job_id.as_str());
    assert_eq!(second.json["error"]["details"]["repo"], "sample");

    drop(permit);
    let job = poll_job(&f, &job_id).await;
    assert_eq!(job["status"], "done", "job: {job}");
    assert_eq!(job["result"]["files_indexed"].as_u64(), Some(1));

    let history = serve_state::send(&f.session, "GET", "/api/v1/index/runs", None).await;
    assert_eq!(history.status, 200, "body: {}", history.text);
    let items = history.json["data"]["items"]
        .as_array()
        .expect("items array");
    assert_eq!(items.len(), 1, "one recorded run: {}", history.text);
    assert_eq!(items[0]["outcome"], "ok");
    assert_eq!(items[0]["repo"], "sample");
    assert!(
        items[0]["files_indexed"].as_u64().unwrap_or(0) > 0,
        "row: {}",
        items[0]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mode_full_rewalks_every_file_where_a_second_incremental_run_walks_none() {
    let f = fixture(false);
    let path = repo_path(&f);

    let first = run_to_completion(&f, json!({"repo": "sample", "path": path})).await;
    assert_eq!(first["result"]["files_indexed"].as_u64(), Some(1));

    let second = run_to_completion(&f, json!({"repo": "sample", "path": path})).await;
    assert_eq!(
        second["result"]["files_indexed"].as_u64(),
        Some(0),
        "an unchanged incremental run re-walks nothing"
    );

    let full = run_to_completion(&f, json!({"repo": "sample", "path": path, "mode": "full"})).await;
    assert_eq!(
        full["result"]["files_indexed"].as_u64(),
        Some(1),
        "full clears the blob-OID cursor first"
    );
    assert_eq!(full["result"]["mode"], "full");

    let history = serve_state::send(
        &f.session,
        "GET",
        "/api/v1/index/runs?repo=sample&limit=10",
        None,
    )
    .await;
    let items = history.json["data"]["items"]
        .as_array()
        .expect("items array");
    assert_eq!(items.len(), 3, "one row per run: {}", history.text);
    assert_eq!(items[0]["mode"], "full", "newest first");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_root_alias_is_accepted_and_an_ambiguous_or_absent_root_is_400() {
    let f = fixture(false);
    let path = repo_path(&f);

    let none = post_run(&f, json!({"repo": "sample"})).await;
    assert_eq!(none.status, 400, "body: {}", none.text);
    assert_eq!(none.json["error"]["code"], "bad_request");

    let many = post_run(
        &f,
        json!({"repo": "sample", "paths": [path.clone(), path.clone()]}),
    )
    .await;
    assert_eq!(many.status, 400, "body: {}", many.text);
    assert!(
        many.json["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("one root per run"),
        "body: {}",
        many.text
    );

    let both = post_run(&f, json!({"repo": "sample", "path": path, "root": path})).await;
    assert_eq!(both.status, 400, "body: {}", both.text);

    let single = run_to_completion(&f, json!({"repo": "sample", "paths": [path.clone()]})).await;
    assert_eq!(single["result"]["files_indexed"].as_u64(), Some(1));

    let alias = run_to_completion(&f, json!({"repo": "sample", "root": path})).await;
    assert_eq!(alias["result"]["repo"], "sample");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_root_outside_every_allowed_root_never_starts_a_job() {
    let f = fixture(false);
    let outside = TempDir::new().expect("outside");

    let resp = post_run(
        &f,
        json!({"repo": "sample", "path": outside.path().to_str().expect("utf8 path")}),
    )
    .await;

    assert_eq!(resp.status, 403, "body: {}", resp.text);
    assert_eq!(resp.json["error"]["code"], "forbidden");
    let jobs = serve_state::send(&f.session, "GET", "/api/v1/jobs", None).await;
    assert_eq!(
        jobs.json["data"]["items"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default(),
        0,
        "a refused request must leave no job behind: {}",
        jobs.text
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_archived_repo_refuses_a_new_run() {
    let f = fixture(false);
    let path = repo_path(&f);
    {
        let conn = f.state.conn().expect("conn");
        conn.execute(
            "INSERT INTO repo_marker(repo, root_path, archived) VALUES ('sample', ?1, 1)",
            [&path],
        )
        .expect("seed archived marker");
    }

    let resp = post_run(&f, json!({"repo": "sample", "path": path})).await;

    assert_eq!(resp.status, 400, "body: {}", resp.text);
    assert!(
        resp.json["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("archived"),
        "body: {}",
        resp.text
    );
}

#[tokio::test]
async fn a_read_only_server_refuses_a_run_and_still_serves_the_history() {
    let f = fixture(true);
    let path = repo_path(&f);

    let refused = post_run(&f, json!({"repo": "sample", "path": path})).await;
    assert_eq!(refused.status, 405, "body: {}", refused.text);
    assert_eq!(refused.json["error"]["code"], "read_only");

    let history = serve_state::send(&f.session, "GET", "/api/v1/index/runs", None).await;
    assert_eq!(history.status, 200, "body: {}", history.text);
    assert_eq!(history.json["data"]["total"].as_u64(), Some(0));
    assert_eq!(history.json["data"]["has_more"], false);
}
