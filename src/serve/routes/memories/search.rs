//! `GET|POST /api/v1/memories/search` (`retrieval::search`) and
//! `GET|POST /api/v1/context` (`retrieval::context`). `GET` takes query params
//! (no vector — a 1024-float embedding does not fit in a query string);
//! `POST` takes a JSON body and is vector-capable. Both reuse the exact
//! `output::{search,context}::envelope` builders the CLI's `--json` path
//! calls, so the HTTP `data` payload is byte-identical to the CLI shape
//! (just nested one level deeper, inside the `/api/v1` envelope).

use std::time::Instant;

use axum::Router;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::get;
use serde_json::Value;

use crate::domains::retrieval;
use crate::domains::retrieval::scope::ScopeEcho;
use crate::domains::retrieval::{context_result, search_result};
use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::query_or_json::QueryOrJson;
use crate::serve::routes::mutate_response::with_lazy_ctx;
use crate::serve::routes::{respond, track_for};
use crate::serve::scope::RepoScope;
use crate::utilities::activity::Origin;
use crate::utilities::blocking::run_blocking;

/// This module's routes, merged into the `memories` resource router.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/memories/search",
            get(memories_search).post(memories_search),
        )
        .route("/api/v1/context", get(context).post(context))
}

/// `GET|POST /api/v1/memories/search`. `GET` reads the request from the query
/// string (no vector); `POST` reads it from a JSON body and is
/// vector-capable. Both handlers in this file fold an `X-Comemory-Repo`
/// header into the request's own `repo` filter when it omits one
/// ([`RepoScope`]).
async fn memories_search(
    State(state): State<AppState>,
    scope: RepoScope,
    headers: HeaderMap,
    QueryOrJson(mut req): QueryOrJson<retrieval::search::Request>,
) -> Response {
    req.repo = scope.resolve(req.repo);
    let origin = state.http_origin(&headers);
    handle("search", state, move |state| run_search(state, req, origin)).await
}

/// `GET|POST /api/v1/context`, served the same two ways as [`memories_search`].
async fn context(
    State(state): State<AppState>,
    scope: RepoScope,
    headers: HeaderMap,
    QueryOrJson(mut req): QueryOrJson<retrieval::context::Request>,
) -> Response {
    req.repo = scope.resolve(req.repo);
    let origin = state.http_origin(&headers);
    handle("context", state, move |state| {
        run_context(state, req, origin)
    })
    .await
}

/// Shared spawn-blocking + envelope wiring for the two handlers above.
async fn handle<F>(command: &'static str, state: AppState, f: F) -> Response
where
    F: FnOnce(AppState) -> Result<Value> + Send + 'static,
{
    let started = Instant::now();
    let result = run_blocking(move || f(state)).await;
    respond(command, result, started)
}

fn run_search(state: AppState, req: retrieval::search::Request, origin: Origin) -> Result<Value> {
    let track = track_for(&state)?;
    with_lazy_ctx(&state, |ctx| {
        ctx.origin = origin;
        let result = retrieval::search::run(ctx, req, track)?;
        let envelope = search_result::envelope(
            &result.hits,
            result.query_id.as_deref(),
            result.meta,
            &result.nav,
            state.paths().data_dir(),
            ScopeEcho::of(&result.scope),
        );
        serde_json::to_value(envelope).map_err(Error::Json)
    })
}

fn run_context(state: AppState, req: retrieval::context::Request, origin: Origin) -> Result<Value> {
    let track = track_for(&state)?;
    with_lazy_ctx(&state, |ctx| {
        ctx.origin = origin;
        let result = retrieval::context::run(ctx, req, track)?;
        let envelope = context_result::envelope(
            &result.bundle,
            result.query_id.as_deref(),
            result.meta,
            ScopeEcho::of(&result.scope),
        );
        serde_json::to_value(envelope).map_err(Error::Json)
    })
}
