//! The project writer tools, as their own `#[tool_router]` block: today
//! `project_evidence` (#346), the `domains::projects::evidence_add` core, and
//! `project_propose` (#336), the `domains::projects::propose` core that
//! submits a plan proposal for human review. Each is refused `read_only` on a
//! `--read-only` session before its core runs, takes no repo scope — a
//! project is not repo-scoped — and runs under the session's envelope, like
//! the readers in [`crate::mcp::tools_projects`]. Human-only verbs have no
//! tool here.
//!
//! `description` repeats [`crate::mcp::catalog`] because rmcp's `#[tool]`
//! takes a string LITERAL; `tests/cli_scenario_mcp.rs::mcp_01_lists_catalog`
//! pins the two equal.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ErrorData};
use rmcp::{tool, tool_router};

use crate::domains::projects;
use crate::mcp::server::ComemoryServer;
use crate::mcp::tools_projects::enveloped;
use crate::mcp::tools_write::write_tool;

#[tool_router(router = project_write_router, vis = "pub(crate)")]
impl ComemoryServer {
    /// Record typed evidence against a project or one of its work items.
    #[tool(
        name = "project_evidence",
        description = "Attach typed evidence to a project, or to one work item with workItemId: kind (commit, pull_request, test_run, deployment, session, decision, memory, external_url), source, and externalId, url, repo, commitSha, metadata, criterionIds as the kind needs. Returns the evidence with its trust; reuse idempotencyKey to retry safely."
    )]
    async fn project_evidence(
        &self,
        Parameters(req): Parameters<projects::evidence_add::Request>,
    ) -> Result<CallToolResult, ErrorData> {
        write_tool(self.session(), "project_evidence", move |c, s| {
            enveloped(c, s, req)
        })
        .await
    }
    /// Submit a plan proposal for human review.
    #[tool(
        name = "project_propose",
        description = "Submit a plan proposal for human review: typed operations written against basePlanVersion, with a rationale, assumptions and risks. The plan does not change until a human approves it."
    )]
    async fn project_propose(
        &self,
        Parameters(req): Parameters<projects::propose::Request>,
    ) -> Result<CallToolResult, ErrorData> {
        write_tool(self.session(), "project_propose", move |c, s| {
            enveloped(c, s, req)
        })
        .await
    }
}
