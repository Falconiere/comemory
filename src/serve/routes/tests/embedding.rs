#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Queued folder jobs revalidate containment before touching source data.

use std::sync::Arc;
use std::time::Duration;

use comemory::config::{Config, Paths};
use comemory::serve::{AppState, RootOverrides, ServeOptions};
use serde_json::json;
use tempfile::tempdir;

use crate::test_common::serve_state::{self, Session};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_folder_replaced_by_outside_symlink_is_refused() {
    let home = tempdir().unwrap();
    let allowed = tempdir().unwrap();
    let outside = tempdir().unwrap();
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/docs/getting-started.md"),
        outside.path().join("private.md"),
    )
    .unwrap();
    let selected = allowed.path().join("selected");
    std::fs::create_dir(&selected).unwrap();
    let state = AppState::new(
        &Paths::new(home.path()),
        ServeOptions {
            repo: None,
            port: 0,
            read_only: false,
            roots: RootOverrides::new(),
            cfg: Config::defaults(),
            embed_cmd: None,
            allow_path: vec![allowed.path().canonicalize().unwrap()],
        },
    )
    .unwrap();
    let session = Session {
        home,
        token: state.token().to_string(),
        router: comemory::serve::router::build_router(state.clone()),
    };
    let permit = Arc::clone(state.write_permit())
        .acquire_owned()
        .await
        .unwrap();
    let response = serve_state::send(
        &session,
        "POST",
        "/api/v1/run/embedding",
        Some(json!({"path": selected})),
    )
    .await;
    assert_eq!(response.status, 202, "{}", response.text);
    let id = response.json["data"]["job_id"].as_str().unwrap();
    std::fs::remove_dir(&selected).unwrap();
    std::os::unix::fs::symlink(outside.path(), &selected).unwrap();
    drop(permit);
    for _ in 0..1000 {
        let response =
            serve_state::send(&session, "GET", &format!("/api/v1/jobs/{id}"), None).await;
        let job = &response.json["data"];
        if job["status"] == "error" {
            assert_eq!(job["error"]["code"], "forbidden", "{job}");
            let conn = state.conn().unwrap();
            let count: i64 = conn
                .query_row("SELECT count(*) FROM documents", [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0, "outside content must never be indexed");
            return;
        }
        assert_ne!(job["status"], "done", "outside folder was scanned: {job}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("job never failed");
}
