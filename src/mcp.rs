//! `comemory mcp` — the stdio Model Context Protocol adapter, the third
//! delivery surface beside `cli` and `serve`.
//!
//! It runs the same `domains::*::run` cores those two call, so host, terminal
//! and console cannot drift: this layer only resolves scope, refuses what the
//! session forbids, and shapes a core's answer into a protocol result.
//!
//! **stdout belongs to the protocol** — a stray print corrupts the JSON-RPC
//! stream, so diagnostics go to stderr through `tracing`.
//!
//! Owner `delivery::mcp`: never imports `cli` or `serve`.

use std::path::Path;

use rmcp::ServiceExt;

use crate::config::Config;
use crate::config::paths::Paths;
use crate::mcp::server::ComemoryServer;
use crate::mcp::state::McpState;
use crate::prelude::*;

/// The curated tool table: fifteen entries naming the command core each tool
/// runs and whether it writes.
pub mod catalog;
/// The blocking-pool bridge every tool body runs its core through.
pub mod exec;
/// The one MCP-local parameter type (`feedback`).
pub mod params;
/// Command outcomes and crate errors shaped into protocol results.
pub mod result;
/// The session's default repo scope and the per-call resolution rule.
pub mod scope;
/// The rmcp service object: the merged tool router and `get_info`.
pub mod server;
/// Shared per-session state: call gate, paths, config, scope and flags.
pub mod state;
/// The nine read tools.
pub mod tools_read;
/// The two write tools.
pub mod tools_write;

/// Caller-supplied configuration for one `comemory mcp` session — the
/// `serve::ServeOptions` shape minus everything a socket needs (no port, no
/// token, no allow-paths, no embed command).
pub struct McpOptions {
    /// Default repo scope for every tool that accepts a `repo` parameter.
    /// `None` asks [`scope::default_repo`] to derive one from the process's
    /// working directory instead.
    pub repo: Option<String>,
    /// Refuse every mutating tool in [`catalog::TOOLS`] with a tool-level
    /// `read_only` error when true.
    pub read_only: bool,
    /// Layered config (defaults → file → env), threaded to every core.
    pub cfg: Config,
}

/// Run one `comemory mcp` session over stdin/stdout until the host closes
/// the stream, then exit `0`.
///
/// `McpState::new` ensures the data-dir tree and opens `comemory.db`; a
/// store that cannot be opened propagates here and the process exits with
/// the store error on stderr, so the host reports a failed server rather
/// than answering every tool with an error (spec § Failure modes).
///
/// rmcp's own failures — a transport that will not initialize, a service
/// task that panicked — carry no crate `Error` of their own, so both are
/// mapped to [`Error::Other`] with an `mcp:` prefix.
pub async fn serve(paths: &Paths, opts: McpOptions, cwd: &Path) -> Result<()> {
    let server = ComemoryServer::new(McpState::new(paths, opts, cwd)?);
    let running = server
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| Error::Other(format!("mcp: cannot start the stdio service: {e}")))?;
    let reason = running
        .waiting()
        .await
        .map_err(|e| Error::Other(format!("mcp: service task failed: {e}")))?;
    tracing::info!(?reason, "mcp: session closed");
    Ok(())
}
