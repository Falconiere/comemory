//! `comemory context` — headline lookup. Pending Task 11 of the v0.2
//! refactor: this stub keeps the CLI surface stable while the retrieval
//! pipeline is migrated to SQLite + sqlite-vec.

use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::prelude::*;

const EXAMPLES: &str = "\
Examples:
  # Code symbol + linked memories in one round-trip (JSON)
  comemory context run_migration --json";

/// Arguments to `comemory context`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Free-form key — symbol name, file path fragment, or phrase.
    pub key: String,
    /// Graph-walk depth (reserved for Task 17).
    #[arg(long, default_value_t = 1)]
    pub depth: u32,
    /// Maximum number of memory hits to surface (default 5). Must be >= 1.
    #[arg(
        long,
        default_value_t = 5,
        value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..)
    )]
    pub limit: usize,
}

/// Placeholder while Task 11 of the v0.2 plan rewires this entry point
/// against `retrieval::router` and `retrieval::bundle`.
pub async fn run(_a: Args, _json_flag: bool, _data_dir: Option<PathBuf>) -> Result<()> {
    Err(Error::Other(
        "comemory context is being rewired against the v0.2 SQLite store; \
         see Task 11 of the v0.2 plan"
            .into(),
    ))
}
