//! Run a command core that may pause for out-of-process inference, without
//! holding the server's shared connection mutex across the model call (#213).
//!
//! [`super::query_response`] runs a whole command inside one `spawn_blocking`
//! closure, and that closure holds the `MutexGuard` on `AppState`'s single
//! `Connection` for its entire body. A learned reranker inside it would stall
//! every other request, including reads that never touch the reranked query.
//!
//! This helper runs three blocking tasks instead. The guard is taken for the
//! first and the third and is dropped in between, because a
//! `retrieval::staged::Paused` borrows nothing: the scoring call owns its
//! request and the continuation owns its carry-over state.

use std::time::Instant;

use axum::response::Response;
use serde::Serialize;

use crate::domains::retrieval::staged::Staged;
use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::routes::{respond, run_blocking};
use crate::utilities::context::Ctx;

/// Run a staged command core and envelope its owned result.
///
/// Phase one and phase three each take the shared connection; phase two holds
/// nothing at all. A core that never pauses (`[rerank] enabled = false`, the
/// default) takes the lock exactly once and costs the same as
/// [`super::query_response`].
pub(crate) async fn staged_query_response<T, B>(
    state: AppState,
    command: &str,
    begin: B,
) -> Response
where
    B: FnOnce(&mut Ctx<'_>) -> Result<Staged<T>> + Send + 'static,
    T: Serialize + Send + 'static,
{
    let started = Instant::now();
    let result = drive(state, begin).await;
    respond(command, result, started)
}

/// The three phases. Split from [`staged_query_response`] so the enveloping
/// stays one line and the lock discipline is readable on its own.
async fn drive<T, B>(state: AppState, begin: B) -> Result<T>
where
    B: FnOnce(&mut Ctx<'_>) -> Result<Staged<T>> + Send + 'static,
    T: Send + 'static,
{
    let phase_one = state.clone();
    let staged = run_blocking(move || with_conn(&phase_one, begin)).await?;
    let paused = match staged {
        Staged::Ready(value) => return Ok(value),
        Staged::Paused(paused) => paused,
    };
    let (call, finish) = paused.into_parts();
    // No guard is held here. This is the whole point of the module: the model
    // runs while unrelated reads keep taking the connection.
    let outcome = run_blocking(move || Ok(call.score())).await?;
    run_blocking(move || with_conn(&state, move |ctx| finish.run(ctx, outcome))).await
}

/// Take the shared connection, build the borrowed `Ctx`, and run `f` under it.
fn with_conn<T>(state: &AppState, f: impl FnOnce(&mut Ctx<'_>) -> Result<T>) -> Result<T> {
    let cfg = state.cfg();
    let mut conn = state.conn()?;
    let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
    f(&mut ctx)
}
