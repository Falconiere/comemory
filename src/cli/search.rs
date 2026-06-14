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

use crate::cli::{embedding_input, load_config, page_meta, page_window, resolve_data_dir};
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
  #   rrf         — pool-normalized relevance in [0,1]
  #   activation  — ACT-R recency boost (post-clamp), neutral = 1.0
  #   feedback    — Beta-smoothed used/irrelevant ratio, neutral = 1.0
  #   quality     — frontmatter quality nudge (1-5 scale), neutral = 1.0
  #   supersede   — 0.2 penalty when superseded by a live memory, else 1.0
  #   final_score — product of all factors (== score at root level)
  # The envelope also carries query_id — the retrieval_log row for this
  # run; pass it to `comemory feedback <query_id> --used <ids>`.
  comemory search \"auth race\" --json

  # Caller-supplied vector (BYO-vector, CSV form)
  comemory search \"advisory lock\" --vector 0.1,0.2,0.3,...";

/// Arguments to `comemory search`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Natural-language query string.
    pub query: String,
    /// Page size — overrides the configured `retrieval.top_k`. `--limit`
    /// is an accepted alias. `0` means "all remaining within the
    /// `max_page_window`".
    #[arg(long, visible_alias = "limit")]
    pub k: Option<usize>,
    /// Number of leading ranked results to skip (deep paging). Bounded by
    /// `retrieval.max_page_window`; once the window ceiling is reached
    /// `has_more` is false and deeper results require refining the query.
    #[arg(long, default_value_t = 0)]
    pub offset: usize,
    /// Optional repo filter forwarded to the vector branch.
    #[arg(long)]
    pub repo: Option<String>,
    /// Filter results to one memory kind.
    #[arg(long)]
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
    let cfg = load_config(&paths)?;
    let window = page_window(&cfg, a.k, a.offset);
    let run = pipeline::search(
        &cfg,
        &conn,
        &a.query,
        vec.as_deref(),
        a.repo.as_deref(),
        a.kind.map(Kind::as_str),
        pipeline::SearchOptions {
            track: true,
            source: crate::stats::source::SEARCH,
            window,
        },
    )?;
    let meta = page_meta(window, run.has_more, run.total);
    output::search::emit(&run.hits, run.query_id.as_deref(), meta, json_flag)
}
