#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! In-process coverage of `src/serve/routes/repos_admin.rs` — connect /
//! patch / archive / disconnect — plus `GET /api/v1/repos`' `"indexing"`
//! overlay. Real temp git repos, the real router, real index jobs; AC-18's
//! disconnect is asserted against real `code_symbols` rows and a real
//! memory that must survive it.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use comemory::config::{Config, Paths};
use comemory::memory::Kind;
use comemory::serve::{AppState, RootOverrides, ServeOptions};
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::test_common::git_sample;
use crate::test_common::serve_state::{self, Session};

/// One in-process session plus the `AppState` behind it (the write permit
/// pins a job in `queued` for the `"indexing"` overlay test).
struct Fixture {
    session: Session,
    state: AppState,
    /// The temp workspace holding `repo` — registered as an allowed root
    /// too, so a `PATCH` can move the repo to a second checkout under it.
    workspace: TempDir,
    repo: PathBuf,
}

fn fixture(read_only: bool) -> Fixture {
    let workspace = TempDir::new().expect("workspace");
    let repo = git_sample::build_sample_repo(workspace.path());
    let home = TempDir::new().expect("home");
    let paths = Paths::new(home.path());
    let mut roots = RootOverrides::new();
    roots.insert("sample".to_string(), repo.clone());
    roots.insert("workspace".to_string(), workspace.path().to_path_buf());
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
        workspace,
        repo,
    }
}

fn repo_path(f: &Fixture) -> String {
    f.repo.to_str().expect("utf8 path").to_string()
}

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

/// Index `sample` through `POST /index/runs` and wait for the job.
async fn index_sample(f: &Fixture) {
    let accepted = serve_state::send(
        &f.session,
        "POST",
        "/api/v1/index/runs",
        Some(json!({"repo": "sample", "path": repo_path(f)})),
    )
    .await;
    assert_eq!(accepted.status, 202, "body: {}", accepted.text);
    let job_id = accepted.json["data"]["job_id"]
        .as_str()
        .expect("job_id")
        .to_string();
    let job = poll_job(f, &job_id).await;
    assert_eq!(job["status"], "done", "job: {job}");
}

/// The one `GET /api/v1/repos` row, by label.
async fn repo_row(f: &Fixture, label: &str) -> Value {
    let resp = serve_state::send(&f.session, "GET", "/api/v1/repos", None).await;
    assert_eq!(resp.status, 200, "body: {}", resp.text);
    resp.json["data"]["repos"]
        .as_array()
        .expect("repos array")
        .iter()
        .find(|row| row["repo"] == label)
        .cloned()
        .unwrap_or(Value::Null)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_registers_a_root_and_the_inventory_reports_it() {
    let f = fixture(false);

    let resp = serve_state::send(
        &f.session,
        "POST",
        "/api/v1/repos",
        Some(json!({"root": repo_path(&f)})),
    )
    .await;

    assert_eq!(resp.status, 200, "body: {}", resp.text);
    assert_eq!(resp.json["data"]["repo"], "sample-repo");
    assert!(resp.json["data"]["root_path"].is_string());
    assert!(
        resp.json["data"]["job_id"].is_null(),
        "no job without index_now: {}",
        resp.text
    );
    let row = repo_row(&f, "sample-repo").await;
    assert_eq!(row["symbols"].as_u64(), Some(0), "row: {row}");
    assert_eq!(row["archived"], false);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_with_index_now_returns_a_job_id_that_indexes_the_repo() {
    let f = fixture(false);

    let resp = serve_state::send(
        &f.session,
        "POST",
        "/api/v1/repos",
        Some(json!({"root": repo_path(&f), "repo": "sample", "index_now": true})),
    )
    .await;

    assert_eq!(resp.status, 200, "body: {}", resp.text);
    let job_id = resp.json["data"]["job_id"]
        .as_str()
        .expect("job_id")
        .to_string();
    let job = poll_job(&f, &job_id).await;
    assert_eq!(job["status"], "done", "job: {job}");
    assert_eq!(job["result"]["files_indexed"].as_u64(), Some(1));
    let row = repo_row(&f, "sample").await;
    assert!(row["symbols"].as_u64().unwrap_or(0) > 0, "row: {row}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn patch_moves_the_root_and_refuses_a_rename_with_501() {
    let f = fixture(false);
    let moved = git_sample::build_sample_repo(&f.workspace.path().join("moved"));
    serve_state::send(
        &f.session,
        "POST",
        "/api/v1/repos",
        Some(json!({"root": repo_path(&f), "repo": "sample"})),
    )
    .await;

    let renamed = serve_state::send(
        &f.session,
        "PATCH",
        "/api/v1/repos/sample",
        Some(json!({"name": "other"})),
    )
    .await;
    assert_eq!(renamed.status, 501, "body: {}", renamed.text);
    assert_eq!(renamed.json["error"]["code"], "unsupported");

    let patched = serve_state::send(
        &f.session,
        "PATCH",
        "/api/v1/repos/sample",
        Some(json!({"root": moved.to_str().expect("utf8 path")})),
    )
    .await;
    assert_eq!(patched.status, 200, "body: {}", patched.text);
    assert!(
        patched.json["data"]["root_path"]
            .as_str()
            .unwrap_or_default()
            .ends_with("sample-repo"),
        "body: {}",
        patched.text
    );

    let unknown = serve_state::send(
        &f.session,
        "PATCH",
        "/api/v1/repos/ghost",
        Some(json!({"root": moved.to_str().expect("utf8 path")})),
    )
    .await;
    assert_eq!(unknown.status, 404, "body: {}", unknown.text);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn archive_flips_the_status_and_un_archive_restores_it() {
    let f = fixture(false);
    index_sample(&f).await;

    let archived = serve_state::send(
        &f.session,
        "POST",
        "/api/v1/repos/sample/archive",
        Some(json!({})),
    )
    .await;
    assert_eq!(archived.status, 200, "body: {}", archived.text);
    assert_eq!(archived.json["data"]["archived"], true);
    let row = repo_row(&f, "sample").await;
    assert_eq!(row["status"], "archived", "row: {row}");

    let restored = serve_state::send(
        &f.session,
        "POST",
        "/api/v1/repos/sample/archive",
        Some(json!({"archived": false})),
    )
    .await;
    assert_eq!(restored.status, 200, "body: {}", restored.text);
    let row = repo_row(&f, "sample").await;
    assert_ne!(row["status"], "archived", "row: {row}");

    let unknown = serve_state::send(
        &f.session,
        "POST",
        "/api/v1/repos/ghost/archive",
        Some(json!({})),
    )
    .await;
    assert_eq!(unknown.status, 404, "body: {}", unknown.text);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnect_needs_confirm_and_then_drops_the_code_index_but_no_memory() {
    let f = fixture(false);
    index_sample(&f).await;
    serve_state::save(
        &f.session,
        "the sample repo is indexed under label sample",
        Kind::Note,
        "sample",
    );

    let unconfirmed = serve_state::send(&f.session, "DELETE", "/api/v1/repos/sample", None).await;
    assert_eq!(unconfirmed.status, 400, "body: {}", unconfirmed.text);
    assert_eq!(
        unconfirmed.json["error"]["code"], "confirmation_required",
        "body: {}",
        unconfirmed.text
    );
    assert!(
        repo_row(&f, "sample").await["symbols"]
            .as_u64()
            .unwrap_or(0)
            > 0,
        "an unconfirmed delete changes nothing"
    );

    let dropped = serve_state::send(
        &f.session,
        "DELETE",
        "/api/v1/repos/sample?confirm=true",
        None,
    )
    .await;
    assert_eq!(dropped.status, 200, "body: {}", dropped.text);
    assert!(
        dropped.json["data"]["symbols_removed"]
            .as_u64()
            .unwrap_or(0)
            > 0,
        "body: {}",
        dropped.text
    );
    assert!(
        dropped.json["data"]["files_removed"].as_u64().unwrap_or(0) > 0,
        "body: {}",
        dropped.text
    );

    assert_eq!(
        repo_row(&f, "sample").await,
        Value::Null,
        "the marker row goes with the index"
    );
    let memories = serve_state::send(&f.session, "GET", "/api/v1/memories", None).await;
    assert_eq!(
        memories.json["data"]["items"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default(),
        1,
        "memories are retained (AC-18): {}",
        memories.text
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_repos_overlays_indexing_while_a_job_is_live() {
    let f = fixture(false);
    index_sample(&f).await;
    let permit = Arc::clone(f.state.write_permit())
        .acquire_owned()
        .await
        .expect("write permit");

    let accepted = serve_state::send(
        &f.session,
        "POST",
        "/api/v1/index/runs",
        Some(json!({"repo": "sample", "path": repo_path(&f)})),
    )
    .await;
    assert_eq!(accepted.status, 202, "body: {}", accepted.text);
    let job_id = accepted.json["data"]["job_id"]
        .as_str()
        .expect("job_id")
        .to_string();

    let row = repo_row(&f, "sample").await;
    assert_eq!(row["status"], "indexing", "row: {row}");
    assert_eq!(row["indexing_job"], job_id.as_str());

    drop(permit);
    poll_job(&f, &job_id).await;
    let row = repo_row(&f, "sample").await;
    assert_ne!(row["status"], "indexing", "row: {row}");
    assert!(row["indexing_job"].is_null(), "row: {row}");
}

#[tokio::test]
async fn a_read_only_server_refuses_every_repo_write_with_405() {
    let f = fixture(true);

    for (method, path, body) in [
        (
            "POST",
            "/api/v1/repos",
            Some(json!({"root": repo_path(&f)})),
        ),
        ("PATCH", "/api/v1/repos/sample", Some(json!({}))),
        ("POST", "/api/v1/repos/sample/archive", Some(json!({}))),
        ("DELETE", "/api/v1/repos/sample?confirm=true", None),
    ] {
        let resp = serve_state::send(&f.session, method, path, body).await;
        assert_eq!(resp.status, 405, "{method} {path} body: {}", resp.text);
        assert_eq!(resp.json["error"]["code"], "read_only");
    }
}
