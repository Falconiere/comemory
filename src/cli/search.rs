//! `comemory search` — natural-language search over the v0.2 SQLite store.
//!
//! Resolves the data dir, opens `comemory.db`, parses any caller-supplied
//! vector, then delegates to [`crate::retrieval::pipeline::search`]
//! (route → rerank → diversify → top-k, plus access tracking). When the
//! caller does not supply a vector (`--vector` / `--vector-stdin`), the
//! lexical FTS5 BM25 branch handles the candidate stage — no embedder is
//! loaded.

use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::{embedding_input, load_config, override_top_k, resolve_data_dir};
use crate::config::paths::Paths;
use crate::memory::Kind;
use crate::output;
use crate::prelude::*;
use crate::retrieval::pipeline;
use crate::store::connection;

const EXAMPLES: &str = "\
Examples:
  # Natural-language query, top 12 hits (default); weighted BM25 + priors
  comemory search \"postgres pool exhausted\"

  # Identifier-aware matching — camelCase/snake_case tokens split automatically
  comemory search \"VecDimMismatch\"

  # JSON output; hits[].score_parts breaks down every ranking factor:
  #   rrf         — fused relevance score (RRF/lexical/vector), neutral > 0
  #   activation  — ACT-R recency boost (post-clamp), neutral = 1.0
  #   feedback    — Beta-smoothed used/irrelevant ratio, neutral = 1.0
  #   quality     — frontmatter quality nudge (1-5 scale), neutral = 1.0
  #   supersede   — 0.2 penalty when superseded by a live memory, else 1.0
  #   final_score — product of all factors (== score at root level)
  comemory search \"auth race\" --json

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
    /// Reserved kind filter. Hidden until the router applies it (future
    /// milestone); declared here so callers that pre-bake a flag list keep
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
/// runs the full retrieval pipeline, and emits results in either TTY or
/// JSON form. The `--k` flag overrides `retrieval.top_k`, which the
/// pipeline uses for the final cut.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let conn = connection::open(paths.db_path())?;

    let vec = embedding_input::read_optional(a.vector_stdin, a.vector.as_deref())?;
    let cfg = override_top_k(load_config(&paths)?, a.k);
    let hits = pipeline::search(&cfg, &conn, &a.query, vec.as_deref(), a.repo.as_deref())?;
    output::search::emit(&hits, json_flag)
}
