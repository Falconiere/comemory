//! `GET|POST /api/v1/memories/search` (`retrieval::search`) and
//! `GET|POST /api/v1/context` (`retrieval::context`). `GET` takes query params
//! (no vector — a 1024-float embedding does not fit in a query string);
//! `POST` takes a JSON body and is vector-capable. Both reuse the exact
//! `output::{search,context}::envelope` builders the CLI's `--json` path
//! calls, so the HTTP `data` payload is byte-identical to the CLI shape
//! (just nested one level deeper, inside the `/api/v1` envelope).

use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::Value;

use crate::domains::retrieval;
use crate::domains::retrieval::scope::ScopeEcho;
use crate::domains::retrieval::staged::Staged;
use crate::domains::retrieval::{context_result, search_result};
use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::routes::staged::staged_query_response;
use crate::serve::routes::track_for;
use crate::serve::scope::RepoScope;
use crate::utilities::context::Ctx;

/// This module's routes, merged into the `memories` resource router.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/memories/search",
            get(memories_search_get).post(memories_search_post),
        )
        .route("/api/v1/context", get(context_get).post(context_post))
}

/// Every handler below folds an `X-Comemory-Repo` header into the request's
/// own `repo` filter when the query/body omits one ([`RepoScope`]).
async fn memories_search_get(
    State(state): State<AppState>,
    scope: RepoScope,
    Query(mut req): Query<retrieval::search::Request>,
) -> Response {
    req.repo = scope.resolve(req.repo);
    handle("search", state, move |ctx, state| {
        run_search(ctx, state, req)
    })
    .await
}

async fn memories_search_post(
    State(state): State<AppState>,
    scope: RepoScope,
    Json(mut req): Json<retrieval::search::Request>,
) -> Response {
    req.repo = scope.resolve(req.repo);
    handle("search", state, move |ctx, state| {
        run_search(ctx, state, req)
    })
    .await
}

async fn context_get(
    State(state): State<AppState>,
    scope: RepoScope,
    Query(mut req): Query<retrieval::context::Request>,
) -> Response {
    req.repo = scope.resolve(req.repo);
    handle("context", state, move |ctx, state| {
        run_context(ctx, state, req)
    })
    .await
}

async fn context_post(
    State(state): State<AppState>,
    scope: RepoScope,
    Json(mut req): Json<retrieval::context::Request>,
) -> Response {
    req.repo = scope.resolve(req.repo);
    handle("context", state, move |ctx, state| {
        run_context(ctx, state, req)
    })
    .await
}

/// Shared handler body for the four handlers above, driven through
/// [`staged_query_response`] so an enabled learned ordering stage runs with the
/// shared connection guard released.
async fn handle<F>(command: &'static str, state: AppState, f: F) -> Response
where
    F: FnOnce(&mut Ctx<'_>, AppState) -> Result<Staged<Value>> + Send + 'static,
{
    let handler = state.clone();
    staged_query_response(state, command, move |ctx| f(ctx, handler)).await
}

fn run_search(
    ctx: &mut Ctx<'_>,
    state: AppState,
    req: retrieval::search::Request,
) -> Result<Staged<Value>> {
    let track = track_for(&state)?;
    let data_dir = state.paths().data_dir().to_path_buf();
    retrieval::search::begin(ctx, req, track)?.map(move |result| {
        let envelope = search_result::envelope(
            &result.hits,
            result.query_id.as_deref(),
            result.meta,
            &result.nav,
            &data_dir,
            ScopeEcho::of(&result.scope),
            result.learned.as_ref(),
        );
        serde_json::to_value(envelope).map_err(Error::Json)
    })
}

fn run_context(
    ctx: &mut Ctx<'_>,
    state: AppState,
    req: retrieval::context::Request,
) -> Result<Staged<Value>> {
    let track = track_for(&state)?;
    retrieval::context::begin(ctx, req, track)?.map(|result| {
        let envelope = context_result::envelope(
            &result.bundle,
            result.query_id.as_deref(),
            result.meta,
            ScopeEcho::of(&result.scope),
            result.learned.as_ref(),
        );
        serde_json::to_value(envelope).map_err(Error::Json)
    })
}
