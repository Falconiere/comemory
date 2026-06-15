//! `comemory tui` — launch the read-only interactive terminal explorer.
//!
//! Live lexical search over the memory + code index with a preview pane and
//! optional Memory-tab semantic enrichment. The UI never mutates the store.
//! It renders to the controlling terminal (stderr) and reserves stdout for the
//! Enter-selected id so `id=$(comemory tui)` can capture a pick. `--json` is
//! rejected — the TUI is interactive, not a machine-readable command.

use std::io::IsTerminal;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::prelude::*;
use crate::tui;

const EXAMPLES: &str = "\
Examples:
  # Browse every indexed repo, Memory + Code tabs, lexical live search
  comemory tui

  # Seed the search box and restrict to one repo
  comemory tui --repo myrepo --query \"postgres pool\"

  # Memory-tab semantic enrich (Ctrl-S) via an external embedder
  comemory tui --embed-cmd 'comemory-embed.sh'

  # Capture the Enter-selected id in a shell variable (stdout is reserved)
  id=$(comemory tui)

Keys:
  type / Backspace   edit the query (Ctrl-U clears it)
  Up / Down          move the selection
  PageUp / PageDown  previous / next page
  Tab                switch the Memory / Code tab
  Ctrl-S             Memory-tab semantic enrich (needs an embed command)
  Ctrl-Y             show the selected id on the status line
  Enter              quit and print the selected id to stdout
  Esc / Ctrl-C       quit (prints nothing)";

/// Arguments to `comemory tui`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Restrict search to one repo label (forwarded to both retrieval legs).
    #[arg(long)]
    pub repo: Option<String>,
    /// Seed the search box with an initial query on launch.
    #[arg(long)]
    pub query: Option<String>,
    /// External command to vectorize a query for Memory-tab semantic search.
    /// Reads the query string on stdin, must emit `{"embedding":[<f32>,..]}`
    /// (1024-dim) on stdout. Falls back to `COMEMORY_EMBED_CMD`. Unset →
    /// `Ctrl-S` is a no-op (lexical search still works).
    #[arg(long, env = "COMEMORY_EMBED_CMD")]
    pub embed_cmd: Option<String>,
}

/// Validate that the invocation is interactive, then launch the explorer.
///
/// Rejects `--json` (the TUI has no machine-readable mode) and a non-terminal
/// render channel (stderr) before any terminal takeover, so a piped or
/// scripted call fails cleanly with `EX_CONFIG` instead of emitting escape
/// codes. A piped *stdout* alone is allowed — it carries the selection.
pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    if json {
        return Err(Error::Config(
            "tui is interactive; --json is not supported".into(),
        ));
    }
    if !std::io::stderr().is_terminal() {
        return Err(Error::Config("tui requires an interactive terminal".into()));
    }
    tui::run(a.repo, a.query, a.embed_cmd, data_dir).await
}
