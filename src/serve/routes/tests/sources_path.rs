#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! In-process coverage of `DELETE /api/v1/sources/{target}?confirm=true` —
//! the path form of the existing query-form unindex (spec §6). Real
//! document fixtures registered through `api::index::run`, then removed
//! through the router; the query form is exercised side by side so the two
//! spellings are compared against each other rather than against a
//! hand-written expectation.

use std::path::PathBuf;

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::store::connection;
use tempfile::TempDir;

use crate::test_common::docs_fixtures;
use crate::test_common::serve_state::{self, Session};

/// A session whose data dir already has the four real document fixtures
/// registered and indexed, through the same core `comemory index` runs.
fn seeded_session() -> (Session, TempDir, PathBuf) {
    let session = serve_state::session(false);
    let workspace = TempDir::new().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());
    let paths = Paths::new(session.home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db for seed index");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let out = api::index::run(
        &mut ctx,
        api::index::Request {
            path: vec![docs.to_str().expect("utf8 path").to_string()],
            repo: Some("docs-corpus".into()),
            strict: false,
        },
    )
    .expect("seed index");
    assert_eq!(out.sources.len(), 1);
    (session, workspace, docs)
}

/// The registered source's id, from `GET /api/v1/sources`.
async fn source_id(session: &Session) -> String {
    let resp = serve_state::send(session, "GET", "/api/v1/sources", None).await;
    assert_eq!(resp.status, 200, "body: {}", resp.text);
    resp.json["data"][0]["id"]
        .as_str()
        .expect("source id")
        .to_string()
}

async fn source_count(session: &Session) -> usize {
    let resp = serve_state::send(session, "GET", "/api/v1/sources", None).await;
    resp.json["data"].as_array().map_or(0, Vec::len)
}

/// Percent-encode a filesystem path into ONE URL path segment.
fn as_segment(path: &std::path::Path) -> String {
    path.to_str().expect("utf8 path").replace('/', "%2F")
}

#[tokio::test]
async fn the_path_form_removes_the_same_source_the_query_form_does() {
    let (path_session, _path_workspace, _path_docs) = seeded_session();
    let (query_session, _query_workspace, _query_docs) = seeded_session();
    let id = source_id(&path_session).await;

    let via_path = serve_state::send(
        &path_session,
        "DELETE",
        &format!("/api/v1/sources/{id}?confirm=true"),
        None,
    )
    .await;
    let query_id = source_id(&query_session).await;
    let via_query = serve_state::send(
        &query_session,
        "DELETE",
        &format!("/api/v1/sources?target={query_id}&confirm=true"),
        None,
    )
    .await;

    assert_eq!(via_path.status, 200, "body: {}", via_path.text);
    assert_eq!(via_query.status, 200, "body: {}", via_query.text);
    assert_eq!(
        via_path.json["data"]["documents_removed"], via_query.json["data"]["documents_removed"],
        "the two forms must remove the same rows"
    );
    assert_eq!(
        via_path.json["data"]["documents_removed"].as_u64(),
        Some(docs_fixtures::FIXTURE_COUNT as u64)
    );
    assert_eq!(source_count(&path_session).await, 0);
    assert_eq!(source_count(&query_session).await, 0);
}

#[tokio::test]
async fn the_path_form_accepts_a_percent_encoded_registered_path() {
    let (session, _workspace, docs) = seeded_session();

    let resp = serve_state::send(
        &session,
        "DELETE",
        &format!("/api/v1/sources/{}?confirm=true", as_segment(&docs)),
        None,
    )
    .await;

    assert_eq!(resp.status, 200, "body: {}", resp.text);
    assert!(
        resp.json["data"]["canonical_path"]
            .as_str()
            .unwrap_or_default()
            .ends_with("docs"),
        "body: {}",
        resp.text
    );
    assert_eq!(source_count(&session).await, 0);
}

#[tokio::test]
async fn the_path_form_is_confirm_gated_and_leaves_the_source_alone() {
    let (session, _workspace, _docs) = seeded_session();
    let id = source_id(&session).await;

    let resp = serve_state::send(&session, "DELETE", &format!("/api/v1/sources/{id}"), None).await;

    assert_eq!(resp.status, 400, "body: {}", resp.text);
    assert_eq!(resp.json["error"]["code"], "confirmation_required");
    assert_eq!(source_count(&session).await, 1);
}

#[tokio::test]
async fn an_unknown_target_on_the_path_form_is_404() {
    let (session, _workspace, _docs) = seeded_session();

    let resp = serve_state::send(
        &session,
        "DELETE",
        "/api/v1/sources/deadbeef?confirm=true",
        None,
    )
    .await;

    assert_eq!(resp.status, 404, "body: {}", resp.text);
    assert_eq!(resp.json["error"]["code"], "not_found");
    assert_eq!(source_count(&session).await, 1);
}

#[tokio::test]
async fn a_read_only_server_refuses_the_path_form_with_405() {
    let session = serve_state::session(true);

    let resp = serve_state::send(
        &session,
        "DELETE",
        "/api/v1/sources/anything?confirm=true",
        None,
    )
    .await;

    assert_eq!(resp.status, 405, "body: {}", resp.text);
    assert_eq!(resp.json["error"]["code"], "read_only");
}
