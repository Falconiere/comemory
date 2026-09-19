//! The bridge every tool body runs its command core through.
//!
//! [`run`] hands the whole synchronous call — connection lock included — to
//! [`run_blocking`], so the guard lives and dies inside one blocking task and
//! never crosses an `.await`. Concurrent tool calls serialize on that mutex;
//! nothing can deadlock on it.
//!
//! An [`Access::Write`] refuses a read-only session BEFORE it builds the
//! closure, so a refused write never locks the connection nor runs a line of
//! core. The refusal rides an `Error::Forbidden` carrying `result`'s marker,
//! keeping every tool body one `into_tool_result(run(…).await)`.

use crate::mcp::result;
use crate::mcp::state::McpState;
use crate::prelude::*;
use crate::utilities::blocking::run_blocking;
use crate::utilities::context::Ctx;

/// What a tool is about to do with the store, which decides whether a
/// `--read-only` session lets it run at all.
#[derive(Debug, Clone, Copy)]
pub enum Access {
    /// A read-only core; always runs.
    Read,
    /// A mutating core, named by its tool so the refusal can say which.
    Write(&'static str),
}

/// Run a core against this session's connection. An [`Access::Write`] on a
/// `--read-only` session never calls `f`: the caller gets the `read_only`
/// refusal for that tool instead.
pub async fn run<T, F>(state: McpState, access: Access, f: F) -> Result<T>
where
    F: FnOnce(&mut Ctx<'_>, &McpState) -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    if let Access::Write(tool) = access
        && state.read_only()
    {
        return Err(result::read_only_error(tool));
    }
    run_blocking(move || in_context(&state, f)).await
}

/// Lock the connection, build the borrowed [`Ctx`] around it and call `f`.
/// Runs entirely inside the blocking task [`run`] spawns.
fn in_context<T, F>(state: &McpState, f: F) -> Result<T>
where
    F: FnOnce(&mut Ctx<'_>, &McpState) -> Result<T>,
{
    let mut guard = state.conn()?;
    let mut ctx = Ctx::borrowed(state.paths(), state.cfg(), &mut guard);
    f(&mut ctx, state)
}

#[cfg(test)]
#[path = "tests/exec.rs"]
mod tests;
