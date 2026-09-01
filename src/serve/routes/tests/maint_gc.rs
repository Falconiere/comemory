#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `GET|PUT /api/v1/gc/policy` and `POST /api/v1/gc/run` end to end through
//! the real in-process router (`tests/common/serve_state.rs`) — console-api
//! spec §9 and AC-17's gc half.
//!
//! AC-17 is proven with real data, not a stub clock: a real file is written
//! into `memories/.trash/` and its mtime pushed two days back with
//! `File::set_modified`, so the sweep that reaps it is the same
//! `api::gc::run` mtime comparison a production sweep makes.

use crate::test_common::serve_state::{self, Session};

use serde_json::{Value, json};

/// Poll `GET /jobs/{id}` until the job reaches a terminal status.
async fn wait_for_job(session: &Session, job_id: &str) -> Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_mins(1);
    loop {
        let res = serve_state::send(session, "GET", &format!("/api/v1/jobs/{job_id}"), None).await;
        let data = res.json["data"].clone();
        if matches!(
            data["status"].as_str(),
            Some("done" | "error" | "cancelled")
        ) {
            return data;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "job {job_id} never reached a terminal status: {}",
            res.text
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Put a real file in the session's `memories/.trash/` and back-date its
/// mtime by `days`.
fn trash_file(session: &Session, name: &str, days: u64) -> std::path::PathBuf {
    let dir = session.home.path().join("memories").join(".trash");
    std::fs::create_dir_all(&dir).expect("create trash dir");
    let path = dir.join(name);
    std::fs::write(&path, "---\nid: deadbeef\n---\nbody\n").expect("write trashed file");
    let file = std::fs::File::options()
        .write(true)
        .open(&path)
        .expect("open trashed file");
    let when = std::time::SystemTime::now() - std::time::Duration::from_secs(days * 86_400);
    file.set_modified(when).expect("back-date mtime");
    path
}

// ── GET /api/v1/gc/policy ───────────────────────────────────────────────

#[tokio::test]
async fn policy_reports_the_shipped_windows_on_a_fresh_server() {
    let session = serve_state::session(false);
    let res = serve_state::send(&session, "GET", "/api/v1/gc/policy", None).await;

    assert_eq!(res.status.as_u16(), 200, "body: {}", res.text);
    let data = &res.json["data"];
    assert_eq!(data["trash_retention_days"].as_u64(), Some(30));
    assert_eq!(data["telemetry_retention_days"].as_u64(), Some(90));
    assert!(data["last_run"].is_null());
    assert!(data["last_run_at"].is_null());
}

// ── PUT /api/v1/gc/policy ───────────────────────────────────────────────

/// AC-17 (gc half): `PUT {trash_retention_days: 1}` persists to
/// `config.toml`, the server reloads it, and `POST /gc/run` then reaps a
/// two-day-old trashed file the 30-day default would have kept.
#[tokio::test]
async fn put_policy_persists_and_makes_gc_reap_an_old_trashed_file_ac17() {
    let session = serve_state::session(false);
    let trashed = trash_file(&session, "deadbeef-old-memory.md", 2);

    let res = serve_state::send(
        &session,
        "PUT",
        "/api/v1/gc/policy",
        Some(json!({ "trash_retention_days": 1 })),
    )
    .await;
    assert_eq!(res.status.as_u16(), 200, "body: {}", res.text);
    assert_eq!(res.json["data"]["trash_retention_days"].as_u64(), Some(1));

    let raw =
        std::fs::read_to_string(session.home.path().join("config.toml")).expect("read config.toml");
    assert!(raw.contains("trash_retention_days = 1"), "config: {raw}");

    // The GET reads the reloaded config, not the value the process started with.
    let echo = serve_state::send(&session, "GET", "/api/v1/gc/policy", None).await;
    assert_eq!(echo.json["data"]["trash_retention_days"].as_u64(), Some(1));

    let res = serve_state::send(
        &session,
        "POST",
        "/api/v1/gc/run",
        Some(json!({ "confirm": true })),
    )
    .await;
    assert_eq!(res.status.as_u16(), 202, "body: {}", res.text);
    let job_id = res.json["data"]["job_id"]
        .as_str()
        .expect("job_id")
        .to_string();
    let job = wait_for_job(&session, &job_id).await;

    assert_eq!(job["status"], "done", "job: {job}");
    assert_eq!(job["command"], "gc");
    assert_eq!(job["result"]["removed"].as_u64(), Some(1));
    assert!(!trashed.exists(), "the two-day-old file must be reaped");

    // And the sweep is now the policy's `last_run`.
    let after = serve_state::send(&session, "GET", "/api/v1/gc/policy", None).await;
    assert_eq!(after.json["data"]["last_run"]["removed"].as_u64(), Some(1));
    assert!(after.json["data"]["last_run_at"].is_string());
}

#[tokio::test]
async fn put_policy_rejects_a_zero_window_and_leaves_the_file_untouched_ac17() {
    let session = serve_state::session(false);
    let config = session.home.path().join("config.toml");
    let before = std::fs::read(&config).ok();

    let res = serve_state::send(
        &session,
        "PUT",
        "/api/v1/gc/policy",
        Some(json!({ "trash_retention_days": 0 })),
    )
    .await;
    assert_eq!(res.status.as_u16(), 400, "body: {}", res.text);
    assert_eq!(res.json["error"]["code"], "bad_request");

    let after = std::fs::read(&config).ok();
    assert_eq!(before, after, "a refused PUT must not touch config.toml");

    let echo = serve_state::send(&session, "GET", "/api/v1/gc/policy", None).await;
    assert_eq!(echo.json["data"]["trash_retention_days"].as_u64(), Some(30));
}

#[tokio::test]
async fn put_policy_rejects_an_unknown_field() {
    let session = serve_state::session(false);
    let res = serve_state::send(
        &session,
        "PUT",
        "/api/v1/gc/policy",
        Some(json!({ "retention": 5 })),
    )
    .await;
    assert!(
        res.status.is_client_error(),
        "an unknown field must be refused: {} {}",
        res.status,
        res.text
    );
}

#[tokio::test]
async fn put_policy_is_refused_on_a_read_only_server() {
    let session = serve_state::session(true);
    let res = serve_state::send(
        &session,
        "PUT",
        "/api/v1/gc/policy",
        Some(json!({ "trash_retention_days": 5 })),
    )
    .await;
    assert_eq!(res.status.as_u16(), 405, "body: {}", res.text);
    assert_eq!(res.json["error"]["code"], "read_only");
}

// ── POST /api/v1/gc/run ─────────────────────────────────────────────────

#[tokio::test]
async fn gc_run_without_confirm_is_refused_and_creates_no_job() {
    let session = serve_state::session(false);
    let res = serve_state::send(&session, "POST", "/api/v1/gc/run", Some(json!({}))).await;

    assert_eq!(res.status.as_u16(), 400, "body: {}", res.text);
    assert_eq!(res.json["error"]["code"], "confirmation_required");

    let jobs = serve_state::send(&session, "GET", "/api/v1/jobs", None).await;
    assert_eq!(
        jobs.json["data"]["total"].as_u64(),
        Some(0),
        "{}",
        jobs.text
    );
}

#[tokio::test]
async fn gc_run_is_refused_on_a_read_only_server_before_the_confirm_gate() {
    let session = serve_state::session(true);
    let res = serve_state::send(&session, "POST", "/api/v1/gc/run", Some(json!({}))).await;

    assert_eq!(res.status.as_u16(), 405, "body: {}", res.text);
    assert_eq!(res.json["error"]["code"], "read_only");
}
