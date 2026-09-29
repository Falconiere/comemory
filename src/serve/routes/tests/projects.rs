#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/serve/routes/projects.rs`: the created-project status.
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
use crate::config::{Config, Paths};
use crate::domains::projects::authority::{self, Envelope, Tier};
use crate::domains::projects::create;
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
        serde_json::from_value(json!({"name": key, "keyPrefix": key, "outcome": "o"})).unwrap()
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
