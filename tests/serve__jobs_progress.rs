#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! End-to-end coverage of job progress reporting (AC-33) and the SSE
//! `progress` event's additive compatibility guarantee (AC-34).
//!
//! A real `POST /api/v1/code/index` job runs, in-process (no subprocess,
//! `tower::ServiceExt::oneshot` over the real router — the same style
//! `tests/common/serve_state.rs` uses), over a real, many-file git repo.
//! The listener attaches deterministically — not by racing wall-clock
//! time — by holding `AppState`'s write permit externally before the job
//! is created: a mutating job cannot leave `Queued` until that permit is
//! released, so the SSE stream is guaranteed to start reading while the
//! job is still queued, exactly like `src/serve/jobs/tests/worker.rs`'s
//! `a_mutating_job_waits_for_an_externally_held_write_permit` proves the
//! same gate at the worker level.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::Request as HttpRequest;
use comemory::config::{Config, Paths};
use comemory::serve::{AppState, RootOverrides, ServeOptions};
use tempfile::TempDir;
use tower::ServiceExt;

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

/// Repo label used across this file's tests.
const REPO_LABEL: &str = "bigrepo";

/// How many files the fixture repo carries — enough that the real walk
/// (git2 blob lookups, AST parsing, SQLite writes) reports several
/// distinct progress snapshots rather than jumping straight to `total`.
const FILE_COUNT: usize = 60;

/// Build `<root>/progress-repo` with [`FILE_COUNT`] small Rust files (two
/// functions each), committed in one shot.
fn build_many_file_repo(root: &Path) -> PathBuf {
    let repo = root.join("progress-repo");
    git_repo::init_repo(&repo);
    let entries: Vec<(String, String)> = (0..FILE_COUNT)
        .map(|i| {
            (
                format!("src/mod_{i}.rs"),
                format!("fn f_{i}() {{}}\nfn g_{i}() {{}}\n"),
            )
        })
        .collect();
    let refs: Vec<(&str, &str)> = entries
        .iter()
        .map(|(p, c)| (p.as_str(), c.as_str()))
        .collect();
    git_commit::commit_files(&repo, &refs, "seed many files");
    repo
}

/// A fresh in-process `AppState` with `repo` registered as an allowed
/// `--root` override, so `POST /code/index` can contain it.
fn build_state(home: &TempDir, repo: &Path) -> AppState {
    let paths = Paths::new(home.path());
    let mut roots = RootOverrides::new();
    roots.insert(REPO_LABEL.to_string(), repo.to_path_buf());
    let opts = ServeOptions {
        repo: None,
        port: 0,
        read_only: false,
        roots,
        open: false,
        cfg: Config::defaults(),
        embed_cmd: None,
        allow_path: Vec::new(),
    };
    AppState::new(&paths, opts).expect("AppState::new")
}

/// Send one request through `router` with the session `token` attached,
/// returning `(status, full body text)`. Blocks until the body is fully
/// produced — for an SSE body, that means until the stream itself closes.
async fn send(
    router: &Router,
    token: &str,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> (u16, String) {
    let builder = HttpRequest::builder()
        .method(method)
        .uri(path)
        .header("Host", "127.0.0.1")
        .header("X-Comemory-Token", token);
    let request = if let Some(v) = body {
        builder
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&v).expect("serialize body")))
            .expect("build request")
    } else {
        builder.body(Body::empty()).expect("build request")
    };
    let response = router.clone().oneshot(request).await.expect("oneshot");
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// One parsed SSE event: `(event name, JSON data)`.
fn parse_sse(text: &str) -> Vec<(String, serde_json::Value)> {
    text.split("\n\n")
        .filter(|block| !block.trim().is_empty())
        .map(|block| {
            let mut name = String::new();
            let mut data = String::new();
            for line in block.lines() {
                if let Some(rest) = line.strip_prefix("event: ") {
                    name = rest.to_string();
                } else if let Some(rest) = line.strip_prefix("data: ") {
                    data.push_str(rest);
                }
            }
            let value = serde_json::from_str(&data).unwrap_or(serde_json::Value::Null);
            (name, value)
        })
        .collect()
}

/// Create a real `index-code` job over `repo` on `state`/`router`, attach
/// to its SSE stream while it is still `Queued` (the write permit is held
/// for exactly that purpose), then let it run to completion and return the
/// job id plus the whole captured event-stream text.
///
/// Ordering, made deterministic rather than raced:
/// 1. Hold the write permit — no mutating job can leave `Queued`.
/// 2. `POST /code/index` — accepted immediately (`guard_job` never touches
///    the permit), job sits `Queued`.
/// 3. Spawn the SSE read on its own task and `yield_now` once: the SSE
///    handler's first emission is a synchronous read of the *current*
///    status (still `Queued`) followed by a genuine pending await on both
///    `watch` channels — that whole burst completes within one poll, so
///    one yield is enough to guarantee it has happened.
/// 4. Drop the held permit — the job proceeds to `Running` → `progress`
///    events → `Done`, all captured by the still-reading SSE task.
async fn run_job_and_capture(state: &AppState, router: &Router, repo: &Path) -> (String, String) {
    let token = state.token().to_string();
    let held = Arc::clone(state.write_permit())
        .acquire_owned()
        .await
        .expect("hold the write permit");

    let (status, body) = send(
        router,
        &token,
        "POST",
        "/api/v1/code/index",
        Some(serde_json::json!({
            "repo": REPO_LABEL,
            "path": repo.to_str().expect("utf8 path"),
        })),
    )
    .await;
    assert_eq!(status, 202, "body: {body}");
    let job_id = serde_json::from_str::<serde_json::Value>(&body).expect("json")["data"]["job_id"]
        .as_str()
        .expect("job_id")
        .to_string();

    let events_router = router.clone();
    let events_token = token.clone();
    let events_path = format!("/api/v1/jobs/{job_id}/events");
    let events_task = tokio::spawn(async move {
        send(&events_router, &events_token, "GET", &events_path, None).await
    });
    tokio::task::yield_now().await;
    drop(held);

    let (status, text) = events_task.await.expect("events task");
    assert_eq!(status, 200, "body: {text}");
    (job_id, text)
}

/// AC-33: a real `index-code` job over a real repo emits at least one SSE
/// `progress` event with `done < total` before its terminal event, and
/// `GET /jobs/{id}` reports a non-empty, bounded `log_tail`.
#[tokio::test]
async fn ac33_job_emits_mid_run_progress_and_a_nonempty_log_tail() {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let repo = build_many_file_repo(workspace.path());
    let state = build_state(&home, &repo);
    let router = comemory::serve::router::build_router(state.clone());

    let (job_id, text) = run_job_and_capture(&state, &router, &repo).await;
    let events = parse_sse(&text);

    let terminal_idx = events
        .iter()
        .position(|(name, _)| name == "done" || name == "error")
        .unwrap_or_else(|| panic!("no terminal event in stream: {text}"));
    assert_eq!(
        events[terminal_idx].0, "done",
        "the fixture repo must index cleanly: {text}"
    );

    let has_mid_run_progress = events[..terminal_idx].iter().any(|(name, data)| {
        name == "progress"
            && data["done"].as_u64().unwrap_or(u64::MAX) < data["total"].as_u64().unwrap_or(0)
    });
    assert!(
        has_mid_run_progress,
        "expected a `progress` event with done < total before the terminal event: {text}"
    );

    let token = state.token().to_string();
    let (status, body) = send(
        &router,
        &token,
        "GET",
        &format!("/api/v1/jobs/{job_id}"),
        None,
    )
    .await;
    assert_eq!(status, 200, "body: {body}");
    let get_body: serde_json::Value = serde_json::from_str(&body).expect("json");
    let log_tail = get_body["data"]["log_tail"]
        .as_array()
        .unwrap_or_else(|| panic!("log_tail must be an array: {get_body}"));
    assert!(
        !log_tail.is_empty(),
        "log_tail must be non-empty after indexing real files: {get_body}"
    );
    assert!(
        log_tail.len() <= 20,
        "log_tail must stay bounded to the ring-buffer cap: {get_body}"
    );
}

/// AC-34: an SSE client that ignores `progress` events observes the exact
/// `queued -> running -> done` status sequence it observed before the
/// `progress` event type existed — the compatibility guarantee.
#[tokio::test]
async fn ac34_a_client_ignoring_progress_sees_the_unchanged_status_sequence() {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let repo = build_many_file_repo(workspace.path());
    let state = build_state(&home, &repo);
    let router = comemory::serve::router::build_router(state.clone());

    let (_job_id, text) = run_job_and_capture(&state, &router, &repo).await;
    let events = parse_sse(&text);

    let statuses: Vec<&str> = events
        .iter()
        .filter(|(name, _)| name != "progress")
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(
        statuses,
        vec!["queued", "running", "done"],
        "a client ignoring `progress` must see today's exact sequence: {text}"
    );
}
