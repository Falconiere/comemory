#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `POST /api/v1/projects/{id}/{archive,restore,pause,resume}` (#328) through
//! a real `comemory serve`. The verbs are human-only and a local-mode caller
//! is the local agent, so each answers `403 project_agent_scope` and writes
//! nothing — not even the telemetry row — while a malformed body still
//! answers `400` and a read-only server `405` before authority is consulted.

use assert_cmd::Command;
use serde_json::{Value, json};

#[path = "common/serve_bin.rs"]
mod serve_bin;

use serve_bin::ServeHome;

const VERBS: [&str; 4] = ["archive", "restore", "pause", "resume"];

/// Charter an active-to-be project through the real CLI, as the local
/// operator, in the server's data directory; its id.
fn seed(home: &ServeHome) -> String {
    let out = Command::cargo_bin("comemory")
        .unwrap()
        .env("COMEMORY_DATA_DIR", home.data_dir())
        .args(["--json", "project", "create", "--name", "Lifecycle"])
        .args(["--key-prefix", "LIFE", "--outcome", "Ship it"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let created: Value = serde_json::from_slice(&out.stdout).unwrap();
    created["project"]["id"].as_str().unwrap().to_string()
}

/// `[project_activity_events, project_command_receipts, activity_log]` rows.
fn rows(home: &ServeHome) -> [i64; 3] {
    let db = rusqlite::Connection::open_with_flags(
        home.data_dir().join("comemory.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    [
        "SELECT COUNT(*) FROM project_activity_events",
        "SELECT COUNT(*) FROM project_command_receipts",
        "SELECT COUNT(*) FROM activity_log",
    ]
    .map(|sql| db.query_row(sql, [], |r| r.get(0)).unwrap())
}

fn lifecycle_body(key: &str) -> Value {
    json!({
        "workspaceId": "ignored-by-the-engine",
        "idempotencyKey": key,
        "expectedVersion": 1,
        "reason": "Waiting on review",
    })
}

#[test]
fn the_local_agent_is_refused_every_lifecycle_verb_and_writes_nothing() {
    let home = ServeHome::new();
    let id = seed(&home);
    let before = rows(&home);
    for verb in VERBS {
        let path = format!("/projects/{id}/{verb}");
        let (status, body) = home.post_raw(&path, &lifecycle_body(verb));
        assert_eq!(status, 403, "{verb}: {body}");
        assert_eq!(body["meta"]["command"], format!("project.{verb}"));
        assert_eq!(
            body["error"],
            json!({
                "code": "project_agent_scope",
                "message": "This command requires a signed-in human",
                "details": {"code": "project_agent_scope"},
            })
        );
    }
    assert_eq!(rows(&home), before, "a refused lifecycle verb wrote a row");
    let shown = home.get(&format!("/projects/{id}"));
    assert_eq!(shown["project"]["version"], 1);
    assert_eq!(shown["project"]["status"], "draft");
    assert_eq!(shown["project"]["archivedAt"], Value::Null);
}

#[test]
fn a_malformed_body_answers_400_before_authority() {
    let home = ServeHome::new();
    let id = seed(&home);
    let before = rows(&home);
    let path = format!("/projects/{id}/pause");
    let (status, body) = home.post_raw(&path, &json!({"idempotencyKey": "k"}));
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body["error"]["details"],
        json!({"field": "expectedVersion", "reason": "required"})
    );
    let (status, body) = home.post_text_raw(&path, "[1]".to_string());
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"]["message"], "body must be an object");
    assert_eq!(rows(&home), before);
}

#[test]
fn a_read_only_server_refuses_every_lifecycle_verb() {
    let home = ServeHome::with_args(&["--read-only"]);
    for verb in VERBS {
        let path = format!("/projects/0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f/{verb}");
        let (status, body) = home.post_raw(&path, &lifecycle_body(verb));
        assert_eq!(status, 405, "{verb}: {body}");
        assert_eq!(body["error"]["code"], "read_only");
    }
}
