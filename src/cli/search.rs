//! `comemory search` — natural-language search over the v0.2 SQLite store.
//!
//! Resolves the data dir, opens `comemory.db`, parses any caller-supplied
//! vector, then delegates to [`crate::retrieval::router::route`]. When the
//! caller does not supply a vector (`--vector` / `--vector-stdin`), the
//! lexical FTS5 BM25 branch handles the query — no embedder is loaded.

use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::{embedding_input, override_top_k, resolve_data_dir};
use crate::config::paths::Paths;
use crate::config::Config;
use crate::memory::Kind;
use crate::output;
use crate::prelude::*;
use crate::retrieval::router;
use crate::store::connection;

const EXAMPLES: &str = "\
Examples:
  # Natural-language query, top 12 hits (default)
  comemory search \"postgres migration race\"

  # JSON envelope for piping into other tools
  comemory search \"advisory lock\" --json

  # Caller-supplied vector (BYO-vector, CSV form)
  comemory search \"advisory lock\" --vector 0.1,0.2,0.3,...";

/// Arguments to `comemory search`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Natural-language query string.
    pub query: String,
    /// Override the configured `retrieval.top_k`. Must be >= 1.
    #[arg(
        long,
        value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..)
    )]
    pub k: Option<usize>,
    /// Optional repo filter forwarded to the vector branch.
    #[arg(long)]
    pub repo: Option<String>,
    /// Reserved kind filter. Hidden until the router actually applies it
    /// (Task 12); declared here so callers that pre-bake a flag list keep
    /// parsing without error.
    #[arg(long, hide = true)]
    pub kind: Option<Kind>,
    /// Caller-supplied dense vector as a comma-separated float list.
    #[arg(long)]
    pub vector: Option<String>,
    /// Read a JSON `{ "embedding": [..] }` payload from stdin and use it as
    /// the dense vector for the query.
    #[arg(long, default_value_t = false)]
    pub vector_stdin: bool,
}

/// Run `comemory search`. Opens the DB, resolves the vector input (if any),
/// routes the query, and emits results in either TTY or JSON form.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let conn = connection::open(paths.db_path())?;

    let vec = read_optional_vector(&a)?;
    let cfg = override_top_k(Config::defaults().with_env()?, a.k);
    let hits = router::route(&cfg, &conn, &a.query, vec.as_deref(), a.repo.as_deref())?;
    output::search::emit(&hits, json_flag)
}

/// Resolve the optional caller-supplied vector from `--vector` (CSV) or
/// `--vector-stdin` (JSON). Returns `Ok(None)` when neither flag is set so
/// the lexical-only branch runs.
fn read_optional_vector(args: &Args) -> Result<Option<Vec<f32>>> {
    if args.vector_stdin {
        return Ok(Some(embedding_input::read_stdin_payload()?));
    }
    if let Some(raw) = &args.vector {
        return Ok(Some(embedding_input::parse_csv(raw)?));
    }
    Ok(None)
}
