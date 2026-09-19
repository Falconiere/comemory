//! The two write tools, as one `#[tool_router]` block.
//!
//! [`exec::Access::Write`] refuses both on a `--read-only` session before any
//! core runs. `save` additionally refuses a call that resolved to no repo
//! scope, so a memory can never land under an empty label.
//!
//! `description` repeats [`crate::mcp::catalog`] for the reason
//! [`crate::mcp::tools_read`]'s header gives.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ErrorData};
use rmcp::{tool, tool_router};
use serde::Serialize;

use crate::domains::{learning, memories};
use crate::mcp::params::FeedbackParams;
use crate::mcp::server::ComemoryServer;
use crate::mcp::state::McpState;
use crate::mcp::{exec, result, scope};
use crate::prelude::*;
use crate::utilities::context::Ctx;

#[tool_router(router = write_router, vis = "pub(crate)")]
impl ComemoryServer {
    /// Store a memory under the resolved repo scope.
    #[tool(
        name = "save",
        description = "Store a memory. Returns the new 8-hex id and file path — call \
it for a verified correction, decision or fix with evidence, passing `supersedes` \
with the ids of any memory it replaces."
    )]
    async fn save(
        &self,
        Parameters(mut req): Parameters<memories::save::Request>,
    ) -> Result<CallToolResult, ErrorData> {
        // `--read-only` outranks everything else, as it does for every
        // mutating `/api/v1` route: a read-only session answers `read_only`
        // even to an unscoped call. Then scope is resolved — and refused —
        // BEFORE the store is touched: an unscoped write is a caller error,
        // not a failed save.
        let state = self.session();
        if state.read_only() {
            return Ok(result::read_only("save"));
        }
        req.repo = scope::resolve(Some(req.repo), state.repo()).unwrap_or_default();
        if req.repo.is_empty() {
            return Ok(result::repo_required());
        }
        write_tool(state, "save", move |c, _| {
            memories::save::run(c, req, false, None)
        })
        .await
    }

    /// Record which recalled ids were used, as `implicit` unless the user
    /// stated the verdict ([`FeedbackParams`]).
    #[tool(
        name = "feedback",
        description = "Record which recalled ids you actually used. Returns the \
counts stored against that query_id — call it after every recall you acted on, \
and set confirmed_by_user only when the user stated the verdict."
    )]
    async fn feedback(
        &self,
        Parameters(params): Parameters<FeedbackParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let req = learning::feedback::Request::from(params);
        write_tool(self.session(), "feedback", move |c, _| {
            learning::feedback::run(c, req)
        })
        .await
    }
}

/// The one line both write tools above end in: refuse `tool` outright when
/// the session is `--read-only`, otherwise run `f` on the blocking pool and
/// shape the outcome into a protocol result.
async fn write_tool<T, F>(
    state: McpState,
    tool: &'static str,
    f: F,
) -> Result<CallToolResult, ErrorData>
where
    F: FnOnce(&mut Ctx<'_>, &McpState) -> Result<T> + Send + 'static,
    T: Serialize + Send + 'static,
{
    result::into_tool_result(exec::run(state, exec::Access::Write(tool), f).await)
}
