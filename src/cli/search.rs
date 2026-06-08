//! `comemory search` — natural-language search over the memory index.
//!
//! Pending Task 11 of the v0.2 refactor: this stub keeps the CLI surface
//! stable while the underlying retrieval pipeline is migrated to the new
//! SQLite + sqlite-vec store (Task 10). The full rewrite that wires
//! `retrieval::router` lands in Task 11.

use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::memory::Kind;
use crate::prelude::*;

const EXAMPLES: &str = "\
Examples:
  # Natural-language query, top 12 hits (default)
  comemory search \"postgres migration race\"";

/// Arguments to `comemory search`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Natural-language query string.
    pub query: String,
    /// Maximum number of hits to return (default 12). Must be >= 1.
    #[arg(
        long,
        default_value_t = 12,
        value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..)
    )]
    pub limit: usize,
    /// Optional repo filter.
    #[arg(long)]
    pub repo: Option<String>,
    /// Optional kind filter (decision|bug|...).
    #[arg(long)]
    pub kind: Option<Kind>,
}

/// Placeholder while Task 11 of the v0.2 plan rewires this entry point
/// against `retrieval::router`.
pub async fn run(_a: Args, _json: bool, _data_dir: Option<PathBuf>) -> Result<()> {
    Err(Error::Other(
        "comemory search is being rewired against the v0.2 SQLite store; \
         see Task 11 of the v0.2 plan"
            .into(),
    ))
}
