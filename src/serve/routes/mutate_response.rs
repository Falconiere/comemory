//! Shared mutating-route gates, blocking execution and response envelopes.
//! [`with_ctx`] borrows the shared connection; [`with_lazy_ctx`] opens its own
//! only when needed, keeping provider work outside the shared mutex.
//! Handlers retain validation order by composing these helpers explicitly.

use std::time::Instant;

use axum::response::Response;
use serde::Serialize;

use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::routes::{guard_mutating, respond};
use crate::utilities::activity::Origin;
use crate::utilities::blocking::run_blocking;
use crate::utilities::context::Ctx;

/// Run `body` as the mutating command `command` and envelope its result.
///
/// `405 read_only` and `503 busy` come first, from [`guard_mutating`], before
/// `body` is even built into a task. The write permit it returns is moved
/// into the blocking closure, so it is held for the whole of `body` and
/// released on completion, error or panic alike.
pub(crate) async fn mutate_blocking<T, F>(state: AppState, command: &str, body: F) -> Response
where
    F: FnOnce(&AppState) -> Result<T> + Send + 'static,
    T: Serialize + Send + 'static,
{
    let started = Instant::now();
    let permit = match guard_mutating(command, &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        body(&state)
    })
    .await;
    respond(command, result, started)
}

/// Run `run` against the shared connection, in a [`Ctx`] built over the
/// server's paths and a config snapshot. `origin` replaces the default CLI
/// origin when the route records who called it (the HTTP `User-Agent`).
///
/// Must be called on a blocking-pool thread: it takes the connection lock.
pub(crate) fn with_ctx<T>(
    state: &AppState,
    origin: Option<Origin>,
    run: impl FnOnce(&mut Ctx<'_>) -> Result<T>,
) -> Result<T> {
    let cfg = state.cfg();
    let mut conn = state.conn()?;
    let ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
    let mut ctx = match origin {
        Some(origin) => ctx.with_origin(origin),
        None => ctx,
    };
    run(&mut ctx)
}

/// Run `run` in a [`Ctx::lazy`] over the server's paths and a config snapshot,
/// without taking the shared connection: the core opens the database itself
/// only if it needs it, or never — the memory-store core reads `config.toml`
/// and git, and the `doctor` probe must not create a database on a fresh data
/// dir. The connectionless sibling of [`with_ctx`].
///
/// Must be called on a blocking-pool thread.
pub(crate) fn with_lazy_ctx<T>(
    state: &AppState,
    run: impl FnOnce(&mut Ctx<'_>) -> Result<T>,
) -> Result<T> {
    let cfg = state.cfg();
    run(&mut Ctx::lazy(state.paths(), &cfg))
}

/// [`mutate_blocking`] over [`with_ctx`]: the closure sees only the context,
/// as in [`super::query_response`]. A handler that also needs the state, or
/// that validates before opening the connection, composes the two itself.
pub(crate) async fn mutate_response<T, F>(
    state: AppState,
    command: &str,
    origin: Option<Origin>,
    run: F,
) -> Response
where
    F: FnOnce(&mut Ctx<'_>) -> Result<T> + Send + 'static,
    T: Serialize + Send + 'static,
{
    mutate_blocking(state, command, move |state| with_ctx(state, origin, run)).await
}

#[cfg(test)]
#[path = "tests/mutate_response.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/mutate_origin.rs"]
mod origin_tests;
