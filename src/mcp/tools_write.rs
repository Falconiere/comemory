//! The three write tools, as one `#[tool_router]` block.
//!
//! [`exec::Access::Write`] refuses every write on a `--read-only` session before any
//! core runs. `save` additionally refuses a call that resolved to no repo
//! scope, so a memory can never land under an empty label.
//!
//! `description` repeats [`crate::mcp::catalog`] for the reason
//! [`crate::mcp::tools_read`]'s header gives.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ErrorData};
use rmcp::{tool, tool_router};
use serde::Serialize;

use crate::domains::architecture;
use crate::domains::{learning, memories};
use crate::mcp::params::{ArchitectureSaveParams, FeedbackParams};
use crate::mcp::server::ComemoryServer;
use crate::mcp::state::McpState;
use crate::mcp::{exec, result, scope};
use crate::prelude::*;
use crate::utilities::context::Ctx;

#[tool_router(router = write_router, vis = "pub(crate)")]
impl ComemoryServer {
    /// Validate and store an enriched architecture model.
    #[tool(
        name = "architecture_save",
        description = "Validate and store an enriched architecture model. Supersedes the current model only after every member path validates against the code index."
    )]
    async fn architecture_save(
        &self,
        Parameters(params): Parameters<ArchitectureSaveParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let state = self.session();
        if state.read_only() {
            return Ok(result::read_only("architecture_save"));
        }
        let Some(repo) = scope::resolve(params.repo, state.repo()) else {
            return Ok(result::repo_required());
        };
        write_tool(state, "architecture_save", move |c, _| {
            architecture::save::run(c, &repo, &params.model)
        })
        .await
    }

    /// Store a memory under the resolved repo scope.
    #[tool(
        name = "save",
        description = "Save a verified lesson with evidence. Returns id and path; use supersedes for replaced memories."
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
        description = "Judge returned ids for query_id. Use confirmed_by_user only for explicit user verdicts; skip empty recalls."
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

/// The one line every write tool above ends in: refuse `tool` outright when
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
