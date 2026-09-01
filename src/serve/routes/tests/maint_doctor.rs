#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `GET /api/v1/doctor/system`, `POST /api/v1/doctor/rebuild` and
//! `POST /api/v1/doctor/reembed` end to end through the real in-process
//! router (`tests/common/serve_state.rs`) — console-api spec §8, AC-15 and
//! AC-16's HTTP half.
//!
//! The reembed tests build a session whose `AppState` really carries an
//! `embed_cmd` pointing at a real shell script on disk, so the job runs the
//! same shell-out an operator's `--embed-cmd` would.

use crate::test_common::serve_state::{self, Session};

use comemory::memory::Kind;
use serde_json::{Value, json};
use tempfile::TempDir;

/// Write an executable script under `dir` emitting a `dim`-wide embedding.
fn embed_script(dir: &TempDir, dim: usize) -> String {
    let values = vec!["0.01"; dim].join(",");
    let path = dir.path().join("embed.sh");
    std::fs::write(
        &path,
        format!("#!/bin/sh\ncat >/dev/null\nprintf '{{\"embedding\":[{values}]}}'\n"),
    )
    .expect("write embed script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod embed script");
    }
    path.to_string_lossy().into_owned()
}

/// Poll `GET /jobs/{id}` until the job reaches a terminal status. Bounded
/// by wall-clock, not by iteration count, so a slow runner cannot turn a
/// still-running job into a failure at a different real duration than the
/// sibling helper in `maint_gc.rs` allows.
async fn wait_for_job(session: &Session, job_id: &str) -> Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_mins(1);
    loop {
        let res = serve_state::send(session, "GET", &format!("/api/v1/jobs/{job_id}"), None).await;
        let status = res.json["data"]["status"].as_str().unwrap_or_default();
        if matches!(status, "done" | "error" | "cancelled") {
            return res.json["data"].clone();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "job {job_id} never reached a terminal status: {}",
            res.text
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// `SELECT COUNT(*)` over `table` in the session's own database.
fn count(session: &Session, table: &str) -> i64 {
    let db = session.home.path().join("comemory.db");
    let conn = comemory::store::connection::open(db).expect("open db");
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .expect("count rows")
}

// ── GET /api/v1/doctor/system ───────────────────────────────────────────

#[tokio::test]
async fn system_reports_the_current_schema_on_a_seeded_store_ac15() {
    let session = serve_state::session(false);
    serve_state::save(
        &session,
        "a memory the system facts read counts",
        Kind::Note,
        "demo",
    );

    let res = serve_state::send(&session, "GET", "/api/v1/doctor/system", None).await;
    assert_eq!(res.status.as_u16(), 200, "body: {}", res.text);
    let data = &res.json["data"];
    assert_eq!(data["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        data["schema_version"], data["current_schema_version"],
        "AC-15: a fresh store is at the current schema"
    );
    assert_eq!(
        data["current_schema_version"],
        comemory::store::migrate::CURRENT_VERSION
    );
    assert_eq!(data["markdown_files"].as_u64(), Some(1));
    assert_eq!(data["memory_vec_dim"].as_u64(), Some(1024));
    assert_eq!(data["code_vec_dim"].as_u64(), Some(768));
    assert!(data["db_bytes"].as_u64().unwrap_or_default() > 0);
}

#[tokio::test]
async fn system_stays_available_on_a_read_only_server() {
    let session = serve_state::session(true);
    let res = serve_state::send(&session, "GET", "/api/v1/doctor/system", None).await;
    assert_eq!(res.status.as_u16(), 200, "body: {}", res.text);
}

// ── POST /api/v1/doctor/rebuild ─────────────────────────────────────────

#[tokio::test]
async fn rebuild_alias_accepts_scope_all_and_runs_the_rebuild_job() {
    let session = serve_state::session(false);
    serve_state::save(
        &session,
        "a memory that survives the rebuild",
        Kind::Note,
        "demo",
    );

    let res = serve_state::send(
        &session,
        "POST",
        "/api/v1/doctor/rebuild",
        Some(json!({ "scope": "all", "repo": null, "confirm": true })),
    )
    .await;
    assert_eq!(res.status.as_u16(), 202, "body: {}", res.text);
    let job_id = res.json["data"]["job_id"]
        .as_str()
        .expect("job_id")
        .to_string();

    let job = wait_for_job(&session, &job_id).await;
    assert_eq!(job["status"], "done", "job: {job}");
    assert_eq!(job["command"], "rebuild");
    assert_eq!(count(&session, "memories"), 1);
}

#[tokio::test]
async fn rebuild_alias_refuses_a_per_repo_scope() {
    let session = serve_state::session(false);
    let res = serve_state::send(
        &session,
        "POST",
        "/api/v1/doctor/rebuild",
        Some(json!({ "scope": "repo", "repo": "demo", "confirm": true })),
    )
    .await;
    assert_eq!(res.status.as_u16(), 400, "body: {}", res.text);
    assert!(
        res.json["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("per-repo rebuild is not supported"),
        "body: {}",
        res.text
    );
}

#[tokio::test]
async fn rebuild_alias_keeps_the_confirm_gate() {
    let session = serve_state::session(false);
    let res = serve_state::send(
        &session,
        "POST",
        "/api/v1/doctor/rebuild",
        Some(json!({ "scope": "all" })),
    )
    .await;
    assert_eq!(res.status.as_u16(), 400, "body: {}", res.text);
    assert_eq!(res.json["error"]["code"], "confirmation_required");
}

// ── POST /api/v1/doctor/reembed ─────────────────────────────────────────

#[tokio::test]
async fn reembed_writes_one_memory_vec_row_per_live_memory_ac16() {
    let scripts = TempDir::new().expect("tempdir");
    let cmd = embed_script(&scripts, 1024);
    let session = serve_state::session_with_embed(&cmd);
    serve_state::save(&session, "first memory to re-embed", Kind::Note, "demo");
    serve_state::save(&session, "second memory to re-embed", Kind::Note, "demo");
    assert_eq!(count(&session, "memory_vec"), 0);

    let res = serve_state::send(
        &session,
        "POST",
        "/api/v1/doctor/reembed",
        Some(json!({ "target": "memories" })),
    )
    .await;
    assert_eq!(res.status.as_u16(), 202, "body: {}", res.text);
    let job_id = res.json["data"]["job_id"]
        .as_str()
        .expect("job_id")
        .to_string();

    let job = wait_for_job(&session, &job_id).await;
    assert_eq!(job["status"], "done", "job: {job}");
    assert_eq!(job["command"], "reembed");
    assert_eq!(job["result"]["memories"].as_u64(), Some(2));
    assert_eq!(job["result"]["failed"].as_u64(), Some(0));
    assert_eq!(count(&session, "memory_vec"), 2);
}

#[tokio::test]
async fn reembed_without_an_embed_command_is_503_and_creates_no_job_ac16() {
    let session = serve_state::session(false);
    serve_state::save(&session, "a memory nobody can re-embed", Kind::Note, "demo");

    let res = serve_state::send(
        &session,
        "POST",
        "/api/v1/doctor/reembed",
        Some(json!({ "target": "memories" })),
    )
    .await;
    assert_eq!(res.status.as_u16(), 503, "body: {}", res.text);
    assert_eq!(res.json["error"]["code"], "embedder_unavailable");

    let jobs = serve_state::send(&session, "GET", "/api/v1/jobs", None).await;
    assert_eq!(
        jobs.json["data"]["total"].as_u64(),
        Some(0),
        "no job may be created: {}",
        jobs.text
    );
}

#[tokio::test]
async fn reembed_is_refused_on_a_read_only_server() {
    // This server has no embed command either — read-only must outrank the
    // embedder check, so the answer is 405, not 503.
    let read_only = serve_state::session(true);
    let res = serve_state::send(
        &read_only,
        "POST",
        "/api/v1/doctor/reembed",
        Some(json!({ "target": "memories" })),
    )
    .await;
    assert_eq!(res.status.as_u16(), 405, "body: {}", res.text);
    assert_eq!(res.json["error"]["code"], "read_only");
}
