//! `comemory mcp` — serve the tool interface over stdio for an agent host.
//!
//! The third delivery surface beside `serve` and the terminal: the same
//! command cores, spoken as typed MCP tools. stdout carries JSON-RPC and
//! nothing else, so this module prints no banner — unlike `cli::serve`, whose
//! startup line is a console concern.

use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::load_config;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::mcp::{self, McpOptions};
use crate::prelude::*;

const EXAMPLES: &str = "\
Examples:
  # Register with a host: the host spawns this and talks JSON-RPC on stdio
  comemory mcp

  # Pin every tool's default repo scope instead of deriving it from the cwd
  comemory mcp --repo myrepo

  # Recall only: `save` and `feedback` answer a read_only tool error
  comemory mcp --read-only";

/// Arguments to `comemory mcp`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Default repo label (as passed to `index-code --repo`) for every tool
    /// that accepts a `repo` parameter. Unset derives it from the working
    /// directory's main worktree; an explicit `repo` on a call overrides it.
    #[arg(long, value_name = "NAME")]
    pub repo: Option<String>,
    /// Refuse every mutating tool (`save`, `feedback`) with a tool-level
    /// `read_only` error, and log no tracked recall.
    #[arg(long, default_value_t = false)]
    pub read_only: bool,
}

/// Open the store and run the stdio session until the host closes the
/// stream. `json` is accepted and has no effect: the protocol owns stdout
/// (spec § Interfaces, CLI).
pub async fn run(a: Args, _json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    // `McpState::new` hoists `ensure_dirs()` into its own startup, the way
    // `serve::serve` does; `load_config` tolerates a missing config file.
    let cfg = load_config(&paths)?;
    let cwd = std::env::current_dir().map_err(Error::Io)?;
    let opts = McpOptions {
        repo: a.repo,
        read_only: a.read_only,
        cfg,
    };
    mcp::serve(&paths, opts, &cwd).await
}
