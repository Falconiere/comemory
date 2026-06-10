//! `comemory context` — headline lookup over the v0.2 SQLite store.
//!
//! Runs the query through [`crate::retrieval::pipeline::search`] (the same
//! route → rerank → diversify path as `comemory search`) to surface
//! relevant memory ids, then assembles a [`crate::retrieval::bundle`] that
//! pulls each memory's body and any cross-link edges
//! (`references_file`, `references_symbol`, `relates_to`, `supersedes`)
//! up to depth 2.

use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::{embedding_input, load_config, override_top_k, resolve_data_dir};
use crate::config::paths::Paths;
use crate::output;
use crate::prelude::*;
use crate::retrieval::{bundle, pipeline};
use crate::store::connection;

const EXAMPLES: &str = "\
Examples:
  # Headline lookup for a symbol name, JSON envelope
  comemory context run_migration --json

  # Pin the bundle width to the top 3 hits
  comemory context \"advisory lock\" --k 3

  # ANN-assisted context with a caller-supplied vector
  comemory context \"advisory lock\" --vector 0.1,0.2,...";

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
    /// Caller-supplied dense vector as a comma-separated float list. When
    /// provided together with `query`, both ANN and lexical branches run and
    /// their results are fused via RRF. Without a vector only the lexical
    /// FTS5 path runs.
    #[arg(long)]
    pub vector: Option<String>,
    /// Read a JSON `{ "embedding": [..] }` payload from stdin and use it as
    /// the dense vector for the context lookup. Mutually exclusive with reading
    /// the query from stdin.
    #[arg(long, default_value_t = false)]
    pub vector_stdin: bool,
}

/// Run `comemory context`. Opens the DB, routes the query (with optional
/// vector), then assembles a bundle covering each matched memory plus all
/// cross-link edges walked to depth ≤ 2.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let conn = connection::open(paths.db_path())?;

    let vec = embedding_input::read_optional(a.vector_stdin, a.vector.as_deref())?;
    let cfg = override_top_k(load_config(&paths)?, a.k);
    let hits = pipeline::search(&cfg, &conn, &a.query, vec.as_deref(), a.repo.as_deref())?;
    let ids: Vec<String> = hits.into_iter().map(|h| h.memory_id).collect();
    let bundle = bundle::assemble(&conn, &a.query, &ids)?;
    output::context::emit(&bundle, json_flag)
}
