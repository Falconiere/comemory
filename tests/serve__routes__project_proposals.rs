#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! `/api/v1/projects/{id}/proposals[/{proposalId}]` through a real
//! `comemory serve` (#336). The local agent holds `proposal.create`, so an
//! admitted submission answers `201` and reads back through both `GET`
//! routes. The two refusal tiers stay distinct: 2,001 operations answer the
//! engine's own `400` even in a body over the server's 5 MiB default (never
//! a `413`), while 201 operations or 256 KiB + 1 byte answer `422` naming
//! the cap. A patch identity, a stale base and an unknown proposal answer
//! their codes, and a read-only server refuses the submission with `405`.

use assert_cmd::Command;
use serde_json::{Value, json};

#[path = "common/serve_bin.rs"]
mod serve_bin;

use serve_bin::ServeHome;

const PROJECT: &str = "11111111-1111-4111-8111-111111111111";
const PATH: &str = "/projects/11111111-1111-4111-8111-111111111111/proposals";

/// Charter [`PROJECT`] through the real CLI in the server's data directory.
fn seed(home: &ServeHome) {
    let out = Command::cargo_bin("comemory")
        .unwrap()
        .env("COMEMORY_DATA_DIR", home.data_dir())
        .args([
            "--json",
            "project",
            "create",
            "--id",
            PROJECT,
            "--name",
            "Proposals",
        ])
        .args([
            "--key-prefix",
            "PROP",
            "--outcome",
            "Plans change by review",
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
}

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/projects/proposal_operations.json")).unwrap()
}

/// A submission body of `operations` under `key`; the path names the project.
fn submission(key: &str, operations: Value) -> Value {
    json!({
        "workspaceId": "ignored-by-the-engine", "idempotencyKey": key,
        "basePlanVersion": 0, "operations": operations, "rationale": "Scope the reader"
    })
}

/// A work-item create with an 8,000-character description.
fn large_create(n: usize) -> Value {
    json!({"op": "work_item.create", "workItem": {"kind": "task", "title": "Large",
        "description": "x".repeat(8000), "id": format!("b0000000-0000-4000-8000-{n:012}")}})
}

/// 40 work-item creates whose descriptions pad the serialized operations to
/// exactly `bytes` (key order does not change a compact JSON's length).
fn padded(bytes: usize) -> Value {
    let item = |n: usize, description: String| {
        json!({"op": "work_item.create", "workItem": {"kind": "task", "title": "t",
            "description": description, "id": format!("b0000000-0000-4000-8000-{n:012}")}})
    };
    let bare: Vec<Value> = (0..40).map(|n| item(n, String::new())).collect();
    let mut missing = bytes - Value::from(bare).to_string().len();
    let list: Vec<Value> = (0..40)
        .map(|n| {
            let take = missing.min(8000);
            missing -= take;
            item(n, "x".repeat(take))
        })
        .collect();
    assert_eq!(missing, 0, "not enough room to pad to {bytes}");
    Value::from(list)
}

fn proposals(home: &ServeHome) -> i64 {
    let db = rusqlite::Connection::open_with_flags(
        home.data_dir().join("comemory.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    db.query_row("SELECT COUNT(*) FROM project_plan_proposals", [], |r| {
        r.get(0)
    })
    .unwrap()
}

#[test]
fn an_admitted_submission_answers_201_and_reads_back_through_both_gets() {
    let home = ServeHome::new();
    seed(&home);
    let fixture = fixture();
    let (status, body) = home.post_raw(PATH, &submission("all", fixture["operations"].clone()));
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["meta"]["command"], "project.proposal.submit");
    let proposal = body["data"]["proposal"].clone();
    assert_eq!(proposal["operations"], fixture["normalized"]);
    assert_eq!(proposal["projectId"], PROJECT);
    assert_eq!(proposal["proposerPrincipalId"], "local-agent");

    let id = proposal["id"].as_str().unwrap();
    let shown = home.get(&format!("{PATH}/{id}"));
    assert_eq!(shown["proposal"], proposal);
    let page = home.get_q(PATH, &[("state", "pending"), ("limit", "5")]);
    assert_eq!(page["proposals"][0], proposal);
    assert_eq!(page["nextCursor"], Value::Null);
    let project = home.get(&format!("/projects/{PROJECT}"));
    assert_eq!(project["project"]["status"], "planning");
}

#[test]
fn the_schema_bound_answers_400_even_past_5_mib_and_the_caps_422() {
    let home = ServeHome::new();
    seed(&home);
    let over: Vec<Value> = (0..2001).map(large_create).collect();
    let text = submission("over", Value::from(over)).to_string();
    assert!(text.len() > 5 * 1024 * 1024, "{} bytes", text.len());
    let (status, body) = home.post_text_raw(PATH, text);
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body["error"],
        json!({"code": "invalid_request", "message": "operations is invalid",
               "details": {"field": "operations", "reason": "invalid"}})
    );

    let archive =
        json!({"op": "work_item.archive", "workItemId": "b0000000-0000-4000-8000-000000000001"});
    let (status, body) = home.post_raw(PATH, &submission("count", Value::from(vec![archive; 201])));
    assert_eq!(status, 422, "{body}");
    assert_eq!(
        body["error"]["details"],
        json!({"field": "operations", "reason": "cap_exceeded", "limit": 200, "actual": 201})
    );
    assert_eq!(
        body["error"]["message"],
        "This proposal exceeds the operations limit of 200"
    );

    // Exactly one byte past 256 KiB of serialized operations is refused
    // naming the byte cap; exactly 256 KiB is admitted.
    let (status, body) = home.post_raw(PATH, &submission("bytes-over", padded(262_145)));
    assert_eq!(status, 422, "{body}");
    assert_eq!(
        body["error"]["details"],
        json!({"field": "operations.bytes", "reason": "cap_exceeded",
               "limit": 262_144, "actual": 262_145})
    );
    assert_eq!(proposals(&home), 0);
    let (status, body) = home.post_raw(PATH, &submission("bytes-at", padded(262_144)));
    assert_eq!(status, 201, "{body}");
    assert_eq!(proposals(&home), 1);
}

#[test]
fn a_patch_identity_a_stale_base_and_an_unknown_proposal_answer_their_codes() {
    let home = ServeHome::new();
    seed(&home);
    let id = "b0000000-0000-4000-8000-000000000001";
    let patch = json!([{"op": "work_item.update", "workItemId": id, "patch": {"id": id}}]);
    let (status, body) = home.post_raw(PATH, &submission("patch", patch));
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body["error"]["details"],
        json!({"field": "operations.0", "reason": "invalid"})
    );

    let mut stale = submission(
        "stale",
        json!([{"op": "work_item.archive", "workItemId": id}]),
    );
    stale["basePlanVersion"] = json!(2);
    let (status, body) = home.post_raw(PATH, &stale);
    assert_eq!(status, 409, "{body}");
    assert_eq!(
        body["error"]["details"],
        json!({"code": "proposal_stale", "basePlanVersion": 2, "currentPlanVersion": 0})
    );

    let (status, body) = home.get_raw(&format!("{PATH}/33333333-3333-4333-8333-333333333333"));
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"]["code"], "proposal_not_found");
    let (status, body) = home.get_q_raw(PATH, &[("state", "merged")]);
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body["error"]["details"],
        json!({"field": "state", "reason": "invalid"})
    );
    let (status, body) = home.get_q_raw(PATH, &[("sort", "new")]);
    assert_eq!(status, 400, "{body}");
    let (status, body) = home.get_raw("/projects/99999999-9999-4999-8999-999999999999/proposals");
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"]["code"], "project_not_found");
    // A malformed path id is the schema edge's `400` naming `projectId`, on
    // the submission (the core validates the id it is given) and the reads.
    let archive = json!([{"op": "work_item.archive", "workItemId": id}]);
    let bad = "/projects/not-a-uuid/proposals";
    for (status, body) in [
        home.post_raw(bad, &submission("bad-path", archive)),
        home.get_raw(bad),
        home.get_raw(&format!("{bad}/33333333-3333-4333-8333-333333333333")),
    ] {
        assert_eq!(status, 400, "{body}");
        assert_eq!(
            body["error"]["details"],
            json!({"field": "projectId", "reason": "invalid"})
        );
    }
    assert_eq!(proposals(&home), 0);
}

#[test]
fn a_read_only_server_refuses_the_submission_with_405() {
    let home = ServeHome::with_args(&["--read-only"]);
    seed(&home);
    let archive =
        json!([{"op": "work_item.archive", "workItemId": "b0000000-0000-4000-8000-000000000001"}]);
    let (status, body) = home.post_raw(PATH, &submission("ro", archive));
    assert_eq!(status, 405, "{body}");
    assert_eq!(proposals(&home), 0);
}
