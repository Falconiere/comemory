//! `comemory context` — headline lookup over the v0.2 SQLite store.
//!
//! Routes the query through [`crate::retrieval::router::route`] to surface
//! relevant memory ids, then assembles a [`crate::retrieval::bundle`] that
//! pulls each memory's body and any `references_symbol` edges (and the
//! referenced `code_symbols` rows) in a single round-trip.

use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::{override_top_k, resolve_data_dir};
use crate::config::paths::Paths;
use crate::config::Config;
use crate::output;
use crate::prelude::*;
use crate::retrieval::{bundle, router};
use crate::store::connection;

const EXAMPLES: &str = "\
Examples:
  # Headline lookup for a symbol name, JSON envelope
  comemory context run_migration --json

  # Pin the bundle width to the top 3 hits
  comemory context \"advisory lock\" --k 3";

/// Arguments to `comemory context`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Free-form query — symbol name, file path fragment, or phrase.
    pub query: String,
    /// Override the configured `retrieval.top_k` for this bundle. Must be >= 1.
    #[arg(
        long,
        value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..)
    )]
    pub k: Option<usize>,
    /// Optional repo filter forwarded to the router.
    #[arg(long)]
    pub repo: Option<String>,
}

/// Run `comemory context`. Opens the DB, routes the query, then assembles
/// a bundle covering each matched memory plus any cross-link references.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let conn = connection::open(paths.db_path())?;

    let cfg = override_top_k(Config::defaults().with_env()?, a.k);
    let routed = router::route(&cfg, &conn, &a.query, None, a.repo.as_deref())?;
    let ids: Vec<String> = routed.into_iter().map(|h| h.memory_id).collect();
    let bundle = bundle::assemble(&conn, &a.query, &ids)?;
    output::context::emit(&bundle, json_flag)
}
