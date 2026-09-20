#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! End-to-end coverage of `GET /api/v1/learning/recall-status`
//! (`src/serve/routes/learning_console.rs`) against a real bound server,
//! driving the same save → find → recall-status → feedback → recall-status
//! sequence as `tests/cli__recall_status.rs` and asserting the envelope
//! `data` matches the CLI `--json` shape.

#[path = "common/serve_bin.rs"]
mod serve_bin;

use assert_cmd::cargo::cargo_bin;
use serde_json::json;
use serve_bin::ServeHome;
use std::process::Command;

const REPO: &str = "app";

/// Run `comemory --json recall-status --repo <REPO>` against the same data
/// dir the running server serves, returning its parsed envelope. Mirrors
/// `serve__routes__memories__mod.rs`'s CLI-vs-HTTP parity check: both read
/// the identical `comemory.db`, so the two must answer with the same JSON.
fn cli_recall_status(srv: &ServeHome) -> serde_json::Value {
    let out = Command::new(cargo_bin("comemory"))
        .env("COMEMORY_DATA_DIR", srv.data_dir())
        .args(["--json", "recall-status", "--repo", REPO])
        .output()
        .expect("run comemory recall-status");
    assert!(
        out.status.success(),
        "comemory recall-status failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim())
        .expect("recall-status --json parses")
}

#[test]
fn recall_status_route() {
    let srv = ServeHome::new();

    let saved = srv.post(
        "/memories",
        &json!({ "body": "recall status over http", "kind": "note", "repo": REPO }),
    );
    let memory_id = saved["id"].as_str().expect("save id").to_string();

    let found = srv.get_q("/find", &[("query", "recall status"), ("repo", REPO)]);
    let query_id = found["query_id"]
        .as_str()
        .expect("find query_id")
        .to_string();

    let before = srv.get_q("/learning/recall-status", &[("repo", REPO)]);
    assert_eq!(before["repo"].as_str(), Some(REPO));
    assert_eq!(before["queries"].as_u64(), Some(1));
    assert_eq!(before["feedback_events"].as_u64(), Some(0));
    assert_eq!(before["saves"].as_u64(), Some(1));
    let pending = before["pending"].as_array().expect("pending array");
    assert_eq!(pending.len(), 1, "the tracked query has no verdict yet");
    assert_eq!(pending[0]["query_id"].as_str(), Some(query_id.as_str()));
    assert_eq!(
        before,
        cli_recall_status(&srv),
        "GET /learning/recall-status data must equal `comemory recall-status --json`"
    );

    srv.post(
        "/feedback",
        &json!({ "query_id": query_id, "used": [memory_id] }),
    );

    let after = srv.get_q("/learning/recall-status", &[("repo", REPO)]);
    assert_eq!(after["feedback_events"].as_u64(), Some(1));
    assert!(
        after["pending"]
            .as_array()
            .expect("pending array")
            .is_empty(),
        "the query is judged, no longer pending: {after}"
    );
    assert_eq!(
        after,
        cli_recall_status(&srv),
        "GET /learning/recall-status data must equal `comemory recall-status --json` after feedback"
    );
}
