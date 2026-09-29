//! The two project read tools (#326), as their own `#[tool_router]` block:
//! the `domains::projects` list and show cores, with no repo scope — a
//! project is not repo-scoped. The epic's six project writers (#261) join
//! them in later tasks.
//!
//! `description` repeats [`crate::mcp::catalog`] because rmcp's `#[tool]`
//! takes a string LITERAL; `tests/cli_scenario_mcp.rs::mcp_01_lists_catalog`
//! pins the two equal.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ErrorData};
use rmcp::{tool, tool_router};

use crate::domains::projects;
use crate::mcp::server::ComemoryServer;
use crate::mcp::tools_read::read_tool;

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
        read_tool(self, move |c, _| projects::list::run(c, req)).await
    }

    /// One project charter by id.
    #[tool(
        name = "project_show",
        description = "Read one project charter by UUID: outcome, success criteria, constraints, non-goals, repositories, status, health and current plan version."
    )]
    async fn project_show(
        &self,
        Parameters(req): Parameters<projects::show::Request>,
    ) -> Result<CallToolResult, ErrorData> {
        read_tool(self, move |c, _| projects::show::run(c, req)).await
    }
}
