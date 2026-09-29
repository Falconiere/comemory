#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/serve/routes/projects.rs`: the created-project status
//! and a CLI receipt's replay through the create path (#327).
//! Local mode never admits a create (#315), so the route's `201` is proven by
//! passing a real admitted create — a human envelope over a real temp store —
//! through the same `respond` and `created` the handler uses, while the local
//! agent's refusal keeps its own `403`.

use std::time::Instant;

use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{Value, json};

use super::{CREATE, caller, created};
use crate::cli;
use crate::cli::project::{Args, CreateArgs, ProjectCmd};
use crate::config::{Config, Paths};
use crate::domains::projects::authority::{self, Envelope, Tier};
use crate::domains::projects::create;
use crate::serve::routes::project_request::body;
use crate::serve::routes::respond;
use crate::store::connection;
use crate::utilities::context::Ctx;

/// The response's status and JSON body.
async fn read(response: Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn an_admitted_create_answers_201_and_the_local_agent_its_403() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    paths.ensure_dirs().unwrap();
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let charter = |key: &str| -> create::Request {
        serde_json::from_value(
            json!({"idempotencyKey": key, "name": key, "keyPrefix": key, "outcome": "o"}),
        )
        .unwrap()
    };

    let alice = Envelope::user("alice", Tier::Member);
    let admitted = authority::run(&mut ctx, &alice, charter("SHIP"));
    let (status, body) = read(created(respond(CREATE, admitted, Instant::now()))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["data"]["project"]["createdBy"], "alice");
    assert_eq!(body["data"]["project"]["keyPrefix"], "SHIP");

    let refused = authority::run(&mut ctx, &caller(), charter("NOPE"));
    let (status, body) = read(created(respond(CREATE, refused, Instant::now()))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        body["error"],
        json!({
            "code": "project_agent_scope",
            "message": "This command requires a signed-in human",
            "details": {"code": "project_agent_scope"},
        })
    );
}

/// A create the CLI adapter charted under `ship-1` replays through this
/// handler's own path — raw body, `body`, `authority::run`, `respond`,
/// `created` — as a human: `201`, and `data` is the stored receipt response
/// (the envelope re-keys JSON, so equal as a value; the typed round trip a
/// replay takes reproduces the stored bytes). A changed body under the key is `409 idempotency_conflict`.
/// Local mode's agent cannot reach it until #316 lets HTTP act as a human.
#[tokio::test]
async fn a_cli_receipt_replays_through_the_http_create_path() {
    let dir = tempfile::tempdir().unwrap();
    let flags = CreateArgs {
        name: "Ship".into(),
        key_prefix: "SHIP".into(),
        outcome: "o".into(),
        success_criteria: vec!["Tests pass".into()],
        constraints: Vec::new(),
        non_goals: Vec::new(),
        repositories: vec!["Falconiere/comemory".into()],
        lead_user_id: None,
        target_date: None,
        id: None,
        idempotency_key: Some("ship-1".into()),
    };
    let cmd = ProjectCmd::Create(flags);
    cli::project::run(Args { cmd }, true, Some(dir.path().to_path_buf()))
        .await
        .unwrap();

    let paths = Paths::new(dir.path());
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let stored: String = conn
        .query_row("SELECT response FROM project_command_receipts", [], |r| {
            r.get(0)
        })
        .unwrap();
    let raw = json!({
        "workspaceId": "ignored",
        "idempotencyKey": "ship-1",
        "name": "Ship",
        "keyPrefix": "SHIP",
        "outcome": "o",
        "successCriteria": ["Tests pass"],
        "repositories": ["Falconiere/comemory"],
    });
    let mut post = |raw: &Value| {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let result = body::<create::Request>(raw.to_string().as_bytes())
            .and_then(|req| authority::run(&mut ctx, &Envelope::local_operator(), req));
        created(respond(CREATE, result, Instant::now()))
    };

    let (status, replayed) = read(post(&raw)).await;
    assert_eq!(status, StatusCode::CREATED, "{replayed}");
    let stored_value: Value = serde_json::from_str(&stored).unwrap();
    assert_eq!(replayed["data"], stored_value);
    // The typed round trip a replay takes reproduces the stored bytes.
    let typed: create::Response = serde_json::from_str(&stored).unwrap();
    assert_eq!(serde_json::to_string(&typed).unwrap(), stored);

    let mut changed = raw.clone();
    changed["outcome"] = json!("something else");
    let (status, refused) = read(post(&changed)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(
        refused["error"],
        json!({
            "code": "idempotency_conflict",
            "message": "This idempotency key was already used for a different command",
            "details": {"code": "idempotency_conflict"},
        })
    );
    let projects: i64 = conn
        .query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
        .unwrap();
    assert_eq!(projects, 1);
}
