//! The rmcp service object: the two tool routers composed into one, and the
//! `initialize` answer a host renders.
//!
//! [`ComemoryServer`] holds nothing but the session [`McpState`] and the
//! merged router; every tool body clones that state and hands it to
//! [`crate::mcp::exec`]. The struct is `Clone` because rmcp keeps the service
//! alive across the whole stdio session and each call borrows it.
//!
//! [`INSTRUCTIONS`] is the loop an agent is asked to run — recall, judge,
//! save, verify, retry — and is the one piece of copy a host shows before any
//! tool is called, so it states the loop rather than describing the server.

use rmcp::ServerHandler;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::tool_handler;

use crate::mcp::state::McpState;

/// The five-line loop every session advertises through `get_info`.
///
/// Line 5 names `store_locked` on purpose: it is the one tool-level error
/// class [`crate::mcp::result`] marks retryable, and an agent that does not
/// know it is transient will otherwise report a broken server.
pub const INSTRUCTIONS: &str = "\
1. Recall first: call `find` with the user's question before exploring a repository.
2. Judge what you used: call `feedback` with the ids you acted on, and with confirmed_by_user only when the user stated the verdict.
3. Save what is worth keeping: `save` verified corrections, decisions and fixes with evidence, passing `supersedes` for the memories they replace.
4. Read before citing: `show` an id before you quote or act on it — a search hit carries only an excerpt.
5. Retry a locked store: on a `store_locked` error wait a moment and retry the same call once.";

/// Appended to [`INSTRUCTIONS`] when the session resolved no default repo
/// (no `--repo` and no git work tree at the process's cwd), so the host is
/// told up front why an unqualified `save` will be refused.
const NO_SCOPE_NOTE: &str = "\n\nThis session has no default repo: reads run across every repo, and `save` needs an explicit `repo` parameter.";

/// One `comemory mcp` session, as rmcp sees it.
#[derive(Clone)]
pub struct ComemoryServer {
    /// Connection, paths, config, default scope and the `--read-only` flag.
    state: McpState,
    /// The nine read tools plus the two write tools, merged.
    tool_router: ToolRouter<Self>,
}

impl ComemoryServer {
    /// Compose the read and write routers over `state`.
    pub fn new(state: McpState) -> Self {
        Self {
            state,
            tool_router: Self::read_router() + Self::write_router(),
        }
    }

    /// A clone of the session state, for one tool body to hand to
    /// [`crate::mcp::exec`]. Cloning is cheap — every field is an `Arc` or a
    /// small owned value — and it is what lets a tool body own its state for
    /// the `'static` closure `run_blocking` needs.
    pub fn session(&self) -> McpState {
        self.state.clone()
    }

    /// [`INSTRUCTIONS`], plus the missing-scope sentence when this session
    /// resolved no default repo (spec § Failure modes, row 1).
    fn instructions(&self) -> String {
        match self.state.repo() {
            Some(_) => INSTRUCTIONS.to_string(),
            None => format!("{INSTRUCTIONS}{NO_SCOPE_NOTE}"),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ComemoryServer {
    /// Tools only — no resources, no prompts (spec Non-Goal 2).
    ///
    /// `ServerConfig` is rmcp 3.4's name for the `initialize` result
    /// (`ServerInfo` is a deprecated alias of the same type, and naming it
    /// would trip this crate's `-D warnings`). It is `#[non_exhaustive]`, so
    /// it is built through its constructor and setters rather than a struct
    /// literal with `..`.
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("comemory", env!("CARGO_PKG_VERSION")))
            .with_instructions(self.instructions())
    }
}
