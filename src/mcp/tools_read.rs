//! The twelve read tools, as one `#[tool_router]` block.
//!
//! Each body runs on the blocking pool through [`read_tool`], which shapes
//! the outcome into a protocol result; inside the closure each tool resolves
//! the session's default repo (where it takes one), calls its own `domains::*`
//! core, and builds its structured result inline, since the twelve tools differ
//! in core module,
//! `track()` handling and envelope shape.
//!
//! `description` repeats [`crate::mcp::catalog`] because rmcp's `#[tool]`
//! takes a string LITERAL; `tests/cli_scenario_mcp.rs::mcp_01_lists_catalog`
//! pins the two equal.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ErrorData};
use rmcp::{tool, tool_router};
use serde::Serialize;
use serde_json::json;

use crate::domains::architecture::{check, current, mermaid, scaffold};
use crate::domains::graph::edges_result;
use crate::domains::retrieval::scope::ScopeEcho;
use crate::domains::retrieval::{code_search_result, context_result, search_result};
use crate::domains::{code, graph, learning, memories, retrieval};
use crate::mcp::exec::{self, Access};
use crate::mcp::params::{ArchitectureShapeParams, ArchitectureShowFormat, ArchitectureShowParams};
use crate::mcp::server::ComemoryServer;
use crate::mcp::state::McpState;
use crate::mcp::{result, scope};
use crate::prelude::*;
use crate::utilities::context::Ctx;

#[tool_router(router = read_router, vis = "pub(crate)")]
impl ComemoryServer {
    /// Build a deterministic architecture scaffold from one indexed repo.
    #[tool(
        name = "architecture_scaffold",
        description = "Build a deterministic architecture-model scaffold from the indexed repo. Enrich its names and summaries, then save the complete model."
    )]
    async fn architecture_scaffold(
        &self,
        Parameters(params): Parameters<ArchitectureShapeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let state = self.session();
        let options = params.options();
        let Some(repo) = scope::resolve(params.repo, state.repo()) else {
            return Ok(result::repo_required());
        };
        read_tool(self, move |c, _| scaffold::run(c.conn()?, &repo, &options)).await
    }

    /// Read the current architecture model or its Mermaid rendering.
    #[tool(
        name = "architecture_show",
        description = "Read the current architecture model for a repo as JSON, or request its deterministic Mermaid flowchart source for a human-facing diagram."
    )]
    async fn architecture_show(
        &self,
        Parameters(params): Parameters<ArchitectureShowParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let state = self.session();
        let Some(repo) = scope::resolve(params.repo, state.repo()) else {
            return Ok(result::repo_required());
        };
        read_tool(self, move |c, _| {
            let stored = current::require(c.conn()?, &repo)?;
            match params.format.unwrap_or(ArchitectureShowFormat::Json) {
                ArchitectureShowFormat::Json => {
                    serde_json::to_value(stored.model).map_err(Error::Json)
                }
                ArchitectureShowFormat::Mermaid => {
                    Ok(json!({ "mermaid": mermaid::render(&stored.model) }))
                }
            }
        })
        .await
    }

    /// Report drift between the saved architecture model and today's index.
    #[tool(
        name = "architecture_check",
        description = "Compare the saved architecture model to today's index. Inspect drift before trusting it or after structural repository changes."
    )]
    async fn architecture_check(
        &self,
        Parameters(params): Parameters<ArchitectureShapeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let state = self.session();
        let options = params.options();
        let Some(repo) = scope::resolve(params.repo, state.repo()) else {
            return Ok(result::repo_required());
        };
        read_tool(self, move |c, _| check::run(c.conn()?, &repo, &options)).await
    }

    /// One ranked list across memory, code and documents, plus a `query_id`.
    #[tool(
        name = "find",
        description = "Search memories, code and documents. Returns ranked hits and query_id. Start with k=3; read selected hits, then feedback."
    )]
    async fn find(
        &self,
        Parameters(mut req): Parameters<retrieval::find::Request>,
    ) -> Result<CallToolResult, ErrorData> {
        read_tool(self, move |c, s| {
            req.repo = scope::resolve(req.repo.take(), s.repo());
            let out = retrieval::find::run(c, req, s.track()?)?;
            Ok(json!({
                "hits": out.hits,
                "query_id": out.query_id,
                "limit": out.meta.limit,
                "offset": out.meta.offset,
                "has_more": out.meta.has_more,
                "total": out.meta.total,
            }))
        })
        .await
    }

    /// Ranked memory hits only.
    #[tool(
        name = "search",
        description = "Search memories only; returns ids, titles, scores and query_id. Use k=3 for focused recall."
    )]
    async fn search(
        &self,
        Parameters(mut req): Parameters<retrieval::search::Request>,
    ) -> Result<CallToolResult, ErrorData> {
        read_tool(self, move |c, s| {
            req.repo = scope::resolve(req.repo.take(), s.repo());
            let out = retrieval::search::run(c, req, s.track()?)?;
            let envelope = search_result::envelope(
                &out.hits,
                out.query_id.as_deref(),
                out.meta,
                &out.nav,
                s.paths().data_dir(),
                ScopeEcho::of(&out.scope),
            );
            serde_json::to_value(envelope).map_err(Error::Json)
        })
        .await
    }

    /// Ranked code-symbol hits only.
    #[tool(
        name = "search_code",
        description = "Search indexed code; returns symbols, locations and query_id. Use k=3, then read the relevant files."
    )]
    async fn search_code(
        &self,
        Parameters(mut req): Parameters<retrieval::search_code::Request>,
    ) -> Result<CallToolResult, ErrorData> {
        read_tool(self, move |c, s| {
            req.repo = scope::resolve(req.repo.take(), s.repo());
            let out = retrieval::search_code::run(c, req, s.track()?)?;
            let envelope =
                code_search_result::envelope(&out.hits, out.query_id.as_deref(), out.meta);
            serde_json::to_value(envelope).map_err(Error::Json)
        })
        .await
    }

    /// Full memory bodies and their linked code and relations.
    #[tool(
        name = "context",
        description = "Return full memory bodies and linked code/relations. k limits memory count, not tokens; prefer find then selective show for concise recall."
    )]
    async fn context(
        &self,
        Parameters(mut req): Parameters<retrieval::context::Request>,
    ) -> Result<CallToolResult, ErrorData> {
        read_tool(self, move |c, s| {
            req.repo = scope::resolve(req.repo.take(), s.repo());
            let out = retrieval::context::run(c, req, s.track()?)?;
            let envelope = context_result::envelope(
                &out.bundle,
                out.query_id.as_deref(),
                out.meta,
                ScopeEcho::of(&out.scope),
            );
            serde_json::to_value(envelope).map_err(Error::Json)
        })
        .await
    }

    /// One memory in full. No `repo` parameter: an 8-hex id is global.
    #[tool(
        name = "show",
        description = "Read one memory by its 8-hex id: full body, metadata and relations. Read selected hits before relying on them."
    )]
    async fn show(
        &self,
        Parameters(req): Parameters<memories::show::Request>,
    ) -> Result<CallToolResult, ErrorData> {
        read_tool(self, move |c, _| memories::show::run(c, req)).await
    }

    /// One page of live memories.
    #[tool(
        name = "list",
        description = "Browse a page of live memories; filter by repo, kind, tag or quality."
    )]
    async fn list(
        &self,
        Parameters(mut req): Parameters<memories::list::Request>,
    ) -> Result<CallToolResult, ErrorData> {
        read_tool(self, move |c, s| {
            req.repo = scope::resolve(req.repo.take(), s.repo());
            memories::list::run(c, req)
        })
        .await
    }

    /// Lexical search over relation triplets.
    #[tool(
        name = "edges",
        description = "Search relation triplets by words in titles, paths or relation names. Returns ranked edges; this is not node-id adjacency lookup."
    )]
    async fn edges(
        &self,
        Parameters(req): Parameters<graph::edges::Request>,
    ) -> Result<CallToolResult, ErrorData> {
        // The `edge_fts` self-heal is a write, so a `--read-only` session
        // runs without it — exactly what `GET /api/v1/edges` does.
        read_tool(self, move |c, s| {
            let out = graph::edges::run(c, req, !s.read_only())?;
            let envelope = edges_result::envelope(&out.hits, out.limit, out.offset, out.has_more);
            serde_json::to_value(envelope).map_err(Error::Json)
        })
        .await
    }

    /// The indexed-repository inventory.
    #[tool(
        name = "repos",
        description = "List indexed repo labels, counts and freshness. Unscoped by default; use these labels in repo parameters."
    )]
    async fn repos(
        &self,
        Parameters(req): Parameters<code::repos::Request>,
    ) -> Result<CallToolResult, ErrorData> {
        // UNSCOPED, exactly like `GET /api/v1/repos`: this is the discovery
        // surface an agent uses to learn which labels exist, so the session
        // default must not hide the others. An explicit `repo` still narrows
        // it. `GET /api/v1/repos` adds a job-registry overlay on top; that
        // registry is a `serve`-process concept with no MCP twin.
        read_tool(self, move |c, _s| code::repos::run(c, req)).await
    }

    /// Tracked recalls, verdicts, saves and the still-unjudged queries.
    #[tool(
        name = "recall_status",
        description = "Report shared repo activity since a timestamp: queries, verdicts, saves and unjudged queries. Not session-specific; empty recalls need no verdict."
    )]
    async fn recall_status(
        &self,
        Parameters(mut req): Parameters<learning::recall_status::Request>,
    ) -> Result<CallToolResult, ErrorData> {
        read_tool(self, move |c, s| {
            req.repo = scope::resolve(req.repo.take(), s.repo());
            learning::recall_status::run(c, req)
        })
        .await
    }
}

/// The one line every read tool above is: clone the session, run `f` on the
/// blocking pool with the session locked only inside it, and shape the
/// outcome into a protocol result.
async fn read_tool<T, F>(server: &ComemoryServer, f: F) -> Result<CallToolResult, ErrorData>
where
    F: FnOnce(&mut Ctx<'_>, &McpState) -> Result<T> + Send + 'static,
    T: Serialize + Send + 'static,
{
    result::into_tool_result(exec::run(server.session(), Access::Read, f).await)
}
