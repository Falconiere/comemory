//! `comemory symbol` — semantic search over the code index.
//!
//! Pending Task 13 of the v0.2 refactor: this stub keeps the CLI surface
//! stable while the code-layer search is migrated off LanceDB and onto
//! SQLite + sqlite-vec / SimHash.

use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::prelude::*;

const EXAMPLES: &str = "\
Examples:
  # Exact function-name hit
  comemory symbol run_migration";

/// Arguments to `comemory symbol`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Free-form symbol name (or descriptor) to search for.
    pub name: String,
    /// Maximum number of hits to return (default 5).
    #[arg(long, default_value_t = 5)]
    pub limit: usize,
}

/// Placeholder while Task 13 of the v0.2 plan migrates this entry point
/// onto the SQLite + sqlite-vec / SimHash code-symbol search.
pub async fn run(_a: Args, _json_flag: bool, _data_dir: Option<PathBuf>) -> Result<()> {
    Err(Error::Other(
        "comemory symbol is being rewired against the v0.2 SQLite store; \
         see Task 13 of the v0.2 plan"
            .into(),
    ))
}
