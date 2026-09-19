//! The nine read tools, as one `#[tool_router]` block.
//!
//! Each body runs on the blocking pool through [`read_tool`], which shapes
//! the outcome into a protocol result; inside the closure each tool resolves
//! the session's default repo (where it takes one), calls its own `domains::*`
//! core, and builds the object the matching `/api/v1` route puts in its
//! envelope's `data` — inline, since the nine tools differ in core module,
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

use crate::domains::graph::edges_result;
use crate::domains::retrieval::scope::ScopeEcho;
use crate::domains::retrieval::{code_search_result, context_result, search_result};
use crate::domains::{code, graph, learning, memories, retrieval};
use crate::mcp::exec::{self, Access};
use crate::mcp::server::ComemoryServer;
use crate::mcp::state::McpState;
use crate::mcp::{result, scope};
use crate::prelude::*;
use crate::utilities::context::Ctx;

#[tool_router(router = read_router, vis = "pub(crate)")]
impl ComemoryServer {
    /// One ranked list across memory, code and documents, plus a `query_id`.
    #[tool(
        name = "find",
        description = "Recall memories and code for a natural-language question. \
Returns ranked hits across both corpora plus a query_id — call this first, \
before exploring a repository, and report the verdict back through `feedback`."
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
        description = "Search saved memories only. Returns ranked memory hits with \
their ids, titles and scores plus a query_id; use it when you already know the \
answer is a recorded decision, convention or bug rather than code."
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
        description = "Search indexed code symbols only. Returns ranked functions, \
types and methods with their file, line span and repo plus a query_id; use it to \
locate an implementation before reading files."
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

    /// The token-budgeted briefing bundle for a task.
    #[tool(
        name = "context",
        description = "Assemble a token-budgeted briefing for a task. Returns the \
memories and code symbols that fit the budget, already ordered — call it when \
starting work and you want one payload instead of several searches."
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
        description = "Fetch one memory by its 8-hex id. Returns the full body, \
frontmatter and relations — call it before citing or acting on a hit, since a \
search result carries only an excerpt."
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
        description = "Page through live memories with optional repo, kind, tag and \
quality filters. Returns a page of rows with ids and titles — use it to browse or \
audit what is stored, not to answer a question."
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

    /// Relation-graph neighbours of a node.
    #[tool(
        name = "edges",
        description = "List the graph edges touching a node. Returns each neighbour \
with its relation and direction — use it to follow `supersedes`, `references` and \
co-activation links out from a memory or a file."
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
        description = "List the indexed repositories. Returns each repo label with \
its symbol and memory counts — use it to learn the exact label to pass as `repo` \
elsewhere."
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
        description = "Report the recall loop's state for a repo since a timestamp. \
Returns tracked query counts, verdicts, saves and the pending queries with no \
verdict yet — call it to find which query_ids still owe a `feedback` call."
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
/// blocking pool with the connection locked only inside it, and shape the
/// outcome into a protocol result.
async fn read_tool<T, F>(server: &ComemoryServer, f: F) -> Result<CallToolResult, ErrorData>
where
    F: FnOnce(&mut Ctx<'_>, &McpState) -> Result<T> + Send + 'static,
    T: Serialize + Send + 'static,
{
    result::into_tool_result(exec::run(server.session(), Access::Read, f).await)
}
