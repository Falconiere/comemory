#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Which mutating routes tell their command core who called them.
//!
//! A route that records a row in the activity log hands its core the caller's
//! `User-Agent` as the origin. That is easy to lose when handlers are
//! rewritten onto a shared helper and nothing else would notice, so these
//! cases pin it: each route is called with a recognisable `User-Agent` and the
//! row it left behind must name that agent as the `http` actor. They were
//! written against the hand-written handlers and must hold for the helper.

use comemory::config::Paths;
use comemory::domains::memories::{Kind, id};
use comemory::store::activity::{ActivityFilter, ActivityRow, list};
use comemory::store::connection;
use serde_json::{Value, json};

use crate::test_common::serve_state::{self, Resp, Session};

const AGENT: &str = "origin-probe/1";

/// Call `path` as [`AGENT`].
async fn as_agent(session: &Session, method: &str, path: &str, body: Option<Value>) -> Resp {
    serve_state::send_headers(
        session,
        method,
        path,
        &[
            ("Host", "127.0.0.1"),
            ("X-Comemory-Token", &session.token),
            ("User-Agent", AGENT),
        ],
        body,
    )
    .await
}

/// Every activity row the session's store holds, newest first.
fn rows(session: &Session) -> Vec<ActivityRow> {
    let conn = connection::open(Paths::new(session.home.path()).db_path()).unwrap();
    list(&conn, &ActivityFilter::default(), 0, 0).unwrap().0
}

/// The `(source, actor)` of every row recorded for `command`.
fn origins(session: &Session, command: &str) -> Vec<(String, Option<String>)> {
    rows(session)
        .into_iter()
        .filter(|row| row.command == command)
        .map(|row| (row.source, row.actor))
        .collect()
}

fn agent_over_http() -> (String, Option<String>) {
    ("http".to_string(), Some(AGENT.to_string()))
}

fn ok(resp: &Resp) {
    assert!(resp.status.is_success(), "{} {}", resp.status, resp.text);
}

#[tokio::test]
async fn the_memory_write_routes_record_the_callers_user_agent() {
    let session = serve_state::session(false);
    let body = "origin probe: the write path records its caller";
    let saved = as_agent(
        &session,
        "POST",
        "/api/v1/memories",
        Some(json!({ "body": body, "repo": "demo", "kind": "note" })),
    )
    .await;
    ok(&saved);
    let memory_id = id::memory_id(body);

    ok(&as_agent(
        &session,
        "PATCH",
        &format!("/api/v1/memories/{memory_id}"),
        Some(json!({ "tags": ["origin"] })),
    )
    .await);
    ok(&as_agent(
        &session,
        "DELETE",
        &format!("/api/v1/memories/{memory_id}?confirm=true"),
        None,
    )
    .await);
    ok(&as_agent(
        &session,
        "POST",
        &format!("/api/v1/memories/{memory_id}/restore"),
        None,
    )
    .await);
    ok(&as_agent(
        &session,
        "DELETE",
        &format!("/api/v1/memories/{memory_id}?confirm=true"),
        None,
    )
    .await);
    ok(&as_agent(
        &session,
        "POST",
        &format!("/api/v1/trash/{memory_id}/restore"),
        None,
    )
    .await);

    assert_eq!(origins(&session, "save"), vec![agent_over_http()]);
    assert_eq!(origins(&session, "update"), vec![agent_over_http()]);
    assert_eq!(
        origins(&session, "delete"),
        vec![agent_over_http(), agent_over_http()]
    );
    assert_eq!(
        origins(&session, "restore"),
        vec![agent_over_http(), agent_over_http()],
        "memories restore and trash restore both record the caller"
    );
}

#[tokio::test]
async fn both_feedback_routes_record_the_callers_user_agent() {
    let session = serve_state::session(false);
    let body = "origin probe: pgbouncer pool exhaustion fix";
    serve_state::save(&session, body, Kind::Note, "demo");
    let memory_id = id::memory_id(body);

    let searched = as_agent(
        &session,
        "POST",
        "/api/v1/memories/search",
        Some(json!({ "query": "pgbouncer" })),
    )
    .await;
    ok(&searched);
    let query_id = searched.json["data"]["query_id"]
        .as_str()
        .unwrap()
        .to_string();

    ok(&as_agent(
        &session,
        "POST",
        "/api/v1/feedback",
        Some(json!({ "query_id": query_id, "used": [memory_id] })),
    )
    .await);
    ok(&as_agent(
        &session,
        "POST",
        &format!("/api/v1/search/{query_id}/feedback"),
        Some(json!({ "hit_id": memory_id, "signal": "used" })),
    )
    .await);

    assert_eq!(
        origins(&session, "feedback"),
        vec![agent_over_http(), agent_over_http()],
        "POST /feedback and POST /search/{{query_id}}/feedback both record the caller"
    );
}
