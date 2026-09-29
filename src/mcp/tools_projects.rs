//! The two project read tools (#326), as their own `#[tool_router]` block:
//! the `domains::projects` list core, and the show and plan cores behind
//! `project_show`'s `view` (#335), with no repo scope — a
//! project is not repo-scoped — each run under the session's envelope
//! ([`McpState::project_envelope`]). The epic's six project writers (#261)
//! join them in later tasks.
//!
//! [`McpState::project_envelope`]: crate::mcp::state::McpState::project_envelope
//!
//! `description` repeats [`crate::mcp::catalog`] because rmcp's `#[tool]`
//! takes a string LITERAL; `tests/cli_scenario_mcp.rs::mcp_01_lists_catalog`
//! pins the two equal.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ErrorData};
use rmcp::{tool, tool_router};
use serde::Serialize;

use crate::domains::projects;
use crate::domains::projects::authority::{self, Command};
use crate::mcp::params::{ProjectShowParams, ProjectShowView};
use crate::mcp::server::ComemoryServer;
use crate::mcp::state::McpState;
use crate::mcp::tools_read::read_tool;
use crate::prelude::*;
use crate::utilities::context::Ctx;

#[tool_router(router = project_router, vis = "pub(crate)")]
impl ComemoryServer {
    /// One keyset page of project charters.
    #[tool(
        name = "project_list",
        description = "List project charters newest first, one keyset page at a time. Filter by status, health or includeArchived; pass nextCursor back as cursor for the next page."
    )]
    async fn project_list(
        &self,
        Parameters(req): Parameters<projects::list::Request>,
    ) -> Result<CallToolResult, ErrorData> {
        read_tool(self, move |c, s| enveloped(c, s, req)).await
    }

    /// One project's charter or committed plan by id.
    #[tool(
        name = "project_show",
        description = "Read one project by UUID. Default view charter: outcome, success criteria, constraints, non-goals, repositories, status, health and current plan version. view plan: the committed plan's milestones, work items, criteria and dependencies."
    )]
    async fn project_show(
        &self,
        Parameters(req): Parameters<ProjectShowParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = req.id;
        read_tool(self, move |c, s| match req.view {
            ProjectShowView::Charter => enveloped(c, s, projects::show::Request { id })
                .map(|shown| Shown::Charter(Box::new(shown))),
            ProjectShowView::Plan => {
                enveloped(c, s, projects::plan::Request { id }).map(Shown::Plan)
            }
        })
        .await
    }
}

/// `project_show`'s answer: the core's own response for the chosen view.
#[derive(Serialize)]
#[serde(untagged)]
enum Shown {
    /// `{project}`, boxed: a charter view dwarfs a plan's four vectors.
    Charter(Box<projects::show::Response>),
    /// `{plan}`.
    Plan(projects::plan::Response),
}

/// Run a project core under the session's envelope.
fn enveloped<C: Command>(ctx: &mut Ctx<'_>, state: &McpState, req: C) -> Result<C::Response> {
    authority::run(ctx, state.project_envelope(), req)
}
