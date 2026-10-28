#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `mutate_blocking`, `with_ctx` and `mutate_response` against a real
//! [`AppState`] over a temp data dir, driven through a probe route with
//! `tower::ServiceExt::oneshot`. What every mutating route relies on is
//! asserted here once: the gates answer before the body runs, the write
//! permit is held across the blocking body and released after it, and a
//! failure keeps its enveloped status.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::routing::post;
use comemory::config::{Config, Paths};
use comemory::errors::{Error, Result};
use comemory::serve::routes::mutate_response::{mutate_blocking, mutate_response, with_ctx};
use comemory::serve::{AppState, RootOverrides, ServeOptions};
use comemory::store::activity::{ActivityFilter, list};
use comemory::utilities::activity::Origin;
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

/// A real state over a fresh data dir; the `TempDir` keeps it alive.
fn state(read_only: bool) -> (TempDir, AppState) {
    let home = TempDir::new().unwrap();
    let opts = ServeOptions {
        repo: None,
        port: 0,
        read_only,
        roots: RootOverrides::new(),
        cfg: Config::defaults(),
        embed_cmd: None,
        allow_path: Vec::new(),
    };
    let state = AppState::new(&Paths::new(home.path()), opts).unwrap();
    (home, state)
}

/// A `POST /probe` route whose command body counts its runs and returns
/// `outcome()`, wrapped in `mutate_blocking`.
fn probe_route(state: &AppState, ran: &Arc<AtomicUsize>, outcome: fn() -> Result<Value>) -> Router {
    let ran = Arc::clone(ran);
    Router::new()
        .route(
            "/probe",
            post(move |State(state): State<AppState>| {
                let ran = Arc::clone(&ran);
                async move {
                    mutate_blocking(state, "probe.cmd", move |_state| {
                        ran.fetch_add(1, Ordering::SeqCst);
                        outcome()
                    })
                    .await
                }
            }),
        )
        .with_state(state.clone())
}

async fn send(router: Router) -> (StatusCode, HeaderMap, Value) {
    let request = Request::builder()
        .method("POST")
        .uri("/probe")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, headers, serde_json::from_slice(&bytes).unwrap())
}

fn counter() -> Arc<AtomicUsize> {
    Arc::new(AtomicUsize::new(0))
}

#[tokio::test]
async fn a_successful_command_is_enveloped_with_its_name_and_data() {
    let (_home, state) = state(false);
    let ran = counter();
    let router = probe_route(&state, &ran, || Ok(json!({ "ran": true })));

    let (status, _, body) = send(router).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"], json!({ "ran": true }));
    assert_eq!(body["meta"]["command"], "probe.cmd");
    assert_eq!(ran.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_read_only_server_refuses_before_the_body_runs() {
    let (_home, state) = state(true);
    let ran = counter();
    let router = probe_route(&state, &ran, || Ok(json!(1)));

    let (status, _, body) = send(router).await;

    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(body["error"]["code"], "read_only");
    assert_eq!(body["meta"]["command"], "probe.cmd");
    assert_eq!(ran.load(Ordering::SeqCst), 0, "the body must not have run");
}

#[tokio::test]
async fn a_held_write_permit_answers_busy_then_clears_when_released() {
    let (_home, state) = state(false);
    let ran = counter();
    let router = probe_route(&state, &ran, || Ok(json!(1)));
    let held = state.write_permit().clone().try_acquire_owned().unwrap();

    let (status, headers, body) = send(router.clone()).await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"]["code"], "busy");
    assert_eq!(headers["retry-after"], "5");
    assert_eq!(ran.load(Ordering::SeqCst), 0, "the body must not have run");

    drop(held);
    let (status, _, _) = send(router).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ran.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_core_error_keeps_its_enveloped_status() {
    let (_home, state) = state(false);
    let ran = counter();
    let router = probe_route(&state, &ran, || {
        Err(Error::NotFound("no such thing".into()))
    });

    let (status, _, body) = send(router).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["ok"], false);
    assert_eq!(body["meta"]["command"], "probe.cmd");
}

#[tokio::test]
async fn the_permit_is_held_across_the_blocking_body_and_released_after_it() {
    let (_home, state) = state(false);
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let slow = Router::new()
        .route(
            "/probe",
            post({
                let started_tx = started_tx.clone();
                let release_rx = Arc::new(std::sync::Mutex::new(release_rx));
                move |State(state): State<AppState>| {
                    let started_tx = started_tx.clone();
                    let release_rx = Arc::clone(&release_rx);
                    async move {
                        mutate_blocking(state, "probe.slow", move |_state| {
                            started_tx.send(()).unwrap();
                            release_rx.lock().unwrap().recv().unwrap();
                            Ok(json!("slow done"))
                        })
                        .await
                    }
                }
            }),
        )
        .with_state(state.clone());
    let ran = counter();
    let quick = probe_route(&state, &ran, || Ok(json!("quick done")));

    let first = tokio::spawn(send(slow));
    tokio::task::spawn_blocking(move || started_rx.recv().unwrap())
        .await
        .unwrap();

    let (status, _, body) = send(quick.clone()).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["error"]["code"], "busy");

    release_tx.send(()).unwrap();
    let (status, _, body) = first.await.unwrap();
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"], "slow done");

    let (status, _, body) = send(quick).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the permit was not released: {body}"
    );
}

#[tokio::test]
async fn with_ctx_opens_the_real_migrated_database() {
    let (_home, state) = state(false);

    let total = with_ctx(&state, None, |ctx| {
        let (_rows, total) = list(ctx.conn()?, &ActivityFilter::default(), 0, 0)?;
        Ok(total)
    })
    .unwrap();

    assert_eq!(total, 0, "a fresh store has an empty activity log");
}

#[tokio::test]
async fn with_ctx_defaults_to_the_cli_origin_and_takes_an_http_one_when_given() {
    let (_home, state) = state(false);
    let cfg = Config::defaults();

    let default = with_ctx(&state, None, |ctx| {
        Ok((ctx.origin.source, ctx.origin.actor.clone()))
    })
    .unwrap();
    let http = with_ctx(&state, Some(Origin::http(&cfg, Some("probe/1"))), |ctx| {
        Ok((ctx.origin.source, ctx.origin.actor.clone()))
    })
    .unwrap();

    assert_eq!(default.0, "cli");
    assert_eq!(http, ("http", Some("probe/1".to_string())));
}

#[tokio::test]
async fn mutate_response_hands_the_closure_a_context_and_envelopes_its_result() {
    let (_home, state) = state(false);
    let router = Router::new()
        .route(
            "/probe",
            post(|State(state): State<AppState>| async move {
                mutate_response(state, "probe.ctx", None, |ctx| {
                    let (_rows, total) = list(ctx.conn()?, &ActivityFilter::default(), 0, 0)?;
                    Ok(json!({ "activity_rows": total }))
                })
                .await
            }),
        )
        .with_state(state);

    let (status, _, body) = send(router).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"], json!({ "activity_rows": 0 }));
    assert_eq!(body["meta"]["command"], "probe.ctx");
}
