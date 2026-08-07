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

use crate::api::{self, Ctx};
use crate::cli::search_only::{self, OnlyDomain};
use crate::cli::{embedding_input, load_config, page_window, track_searches, when};
use crate::config::paths::{Paths, resolve_data_dir};
use crate::memory::Kind;
use crate::output;
use crate::prelude::*;
use crate::retrieval::scope::{Domain, Filters};
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
  comemory search \"advisory lock\" --vector 0.1,0.2,0.3,...

  # Time travel: the corpus as it stood on 2026-06-01 — memories created
  # later are excluded, and a hit only counts as superseded if its
  # superseder already existed by then (\"what did we decide back then?\").
  comemory search \"queue backend\" --as-of 2026-06-01 --json

  # Plain created-date window; --until filters candidates only, so a hit
  # superseded *after* the cutoff still shows its present-day penalty.
  # Both bounds accept RFC3339 or a bare YYYY-MM-DD (whole-day inclusive).
  comemory search \"queue backend\" --since 2026-05-01 --until 2026-06-01

  # A hit tagged \"source\": \"graph\" (tier 0) is lexically dark for the
  # query — the graph-expansion leg reached it by walking `edges` out from
  # the top hits. COMEMORY_RETRIEVAL_GRAPH_HOPS bounds the walk depth
  # (default 2, 0 disables the leg); COMEMORY_RETRIEVAL_GRAPH_SEEDS sets
  # how many top hits seed it (default 8).
  COMEMORY_RETRIEVAL_GRAPH_HOPS=0 comemory search \"auth race\" --json";

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
    /// Only search memories created at or after this instant. Accepts an
    /// RFC3339 timestamp or a bare `YYYY-MM-DD` date (start of that UTC day).
    #[arg(long, value_name = "WHEN")]
    pub since: Option<String>,
    /// Only search memories created at or before this instant. Accepts an
    /// RFC3339 timestamp or a bare `YYYY-MM-DD` date (end of that UTC day).
    /// Filters candidates only — the supersede penalty stays present-day.
    #[arg(long, value_name = "WHEN")]
    pub until: Option<String>,
    /// Search the corpus as it stood at this instant: `--until` plus
    /// supersede-penalty scoping, so a hit counts as superseded only by a
    /// memory that already existed then. Same value grammar as `--until`.
    #[arg(long = "as-of", value_name = "WHEN", conflicts_with = "until")]
    pub as_of: Option<String>,
    /// Restrict the query to these domains (repeatable and/or
    /// comma-separated, e.g. `--only memory,document`). Defaults to every
    /// domain. `--kind` implies memory scope; combining it with a
    /// memory-excluding `--only` is a usage error.
    #[arg(long, value_delimiter = ',')]
    pub only: Vec<OnlyDomain>,
    /// Restrict document results to paths matching this Git-style glob
    /// (repeatable; entries are OR'd together). Document-domain only in
    /// v1 — has no effect on memory or code results.
    #[arg(long = "path", value_name = "GLOB")]
    pub path: Vec<String>,
}

/// Run `comemory search`. Opens the DB, resolves the domain scope, and
/// dispatches to the memory pipeline (via `api::search::run`) or — for a
/// scope that excludes memory — the interim document-only path (see
/// [`run_memory`] / `cli::search_only`). The `--k` flag overrides
/// `retrieval.top_k`.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut conn = connection::open(paths.db_path())?;

    let cfg = load_config(&paths)?;
    let window = page_window(&cfg, a.k, a.offset);
    let scope = when::scope_from_flags(a.since.as_deref(), a.until.as_deref(), a.as_of.as_deref())?;
    let kind = a.kind.map(Kind::as_str);
    let domains = search_only::resolve_domains(&a.only, kind)?;
    // s9 fuses the document leg into `pipeline::search`; until then a
    // scope excluding memory (chiefly `--only document`) cannot go
    // through it — that pipeline always runs the full memory leg
    // regardless of `Filters.domains`, and `api::search::Request` carries
    // no `--only`/`--path` fields (see that module's doc). That case
    // takes the interim direct `doc_route` path instead; every other scope
    // (the default, `--only memory`, or any combination that still
    // includes memory) runs unchanged via [`run_memory`].
    if !domains.contains(Domain::Memory) {
        let filters = Filters {
            repo: a.repo.as_deref(),
            kind,
            scope: &scope,
            domains,
        };
        return search_only::run_document_only(
            &conn, &cfg, &a.query, filters, &a.path, window, json_flag,
        );
    }
    run_memory(&a, json_flag, &paths, &mut conn, &cfg).await
}

/// The pre-`--only` memory search path: build the shared `api::search::Request`
/// from the CLI args (reading any `--vector`/`--vector-stdin` payload, a
/// CLI-only affordance) and delegate to `api::search::run`, then emit via
/// `output::search::emit`. Split out of [`run`] to keep it under the
/// function length gate.
async fn run_memory(
    a: &Args,
    json_flag: bool,
    paths: &Paths,
    conn: &mut rusqlite::Connection,
    cfg: &crate::config::Config,
) -> Result<()> {
    let vector = embedding_input::read_optional(a.vector_stdin, a.vector.as_deref())?;
    let req = api::search::Request {
        query: a.query.clone(),
        k: a.k,
        offset: a.offset,
        repo: a.repo.clone(),
        kind: a.kind,
        vector,
        since: a.since.clone(),
        until: a.until.clone(),
        as_of: a.as_of.clone(),
    };
    let mut ctx = Ctx::borrowed(paths, cfg, conn);
    let result = api::search::run(&mut ctx, req, track_searches()?)?;
    output::search::emit(&result, json_flag, paths.data_dir())
}
