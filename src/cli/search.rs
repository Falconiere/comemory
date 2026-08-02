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

use crate::cli::search_only::{self, OnlyDomain};
use crate::cli::{
    embedding_input, load_config, page_meta, page_window, resolve_data_dir, track_searches, when,
};
use crate::config::paths::Paths;
use crate::memory::Kind;
use crate::output;
use crate::prelude::*;
use crate::retrieval::pipeline;
use crate::retrieval::scope::{Domain, Filters};
use crate::store::{connection, memory_meta};

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
/// dispatches to the memory pipeline or — for a scope that excludes
/// memory — the interim document-only path (see [`run_memory`] /
/// `cli::search_only`). The `--k` flag overrides `retrieval.top_k`.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let conn = connection::open(paths.db_path())?;

    let cfg = load_config(&paths)?;
    let window = page_window(&cfg, a.k, a.offset);
    let scope = when::scope_from_flags(a.since.as_deref(), a.until.as_deref(), a.as_of.as_deref())?;
    let kind = a.kind.map(Kind::as_str);
    let domains = search_only::resolve_domains(&a.only, kind)?;
    let filters = Filters {
        repo: a.repo.as_deref(),
        kind,
        scope: &scope,
        domains,
    };
    // s9 fuses the document leg into `pipeline::search`; until then a
    // scope excluding memory (chiefly `--only document`) cannot go
    // through it — that pipeline always runs the full memory leg
    // regardless of `Filters.domains`. That case takes the interim direct
    // `doc_route` path instead; every other scope (the default, `--only
    // memory`, or any combination that still includes memory) runs
    // unchanged via [`run_memory`].
    if !domains.contains(Domain::Memory) {
        return search_only::run_document_only(
            &conn, &cfg, &a.query, filters, &a.path, window, json_flag,
        );
    }
    run_memory(&a, json_flag, &paths, &conn, &cfg, filters, window).await
}

/// The pre-`--only` memory search path, unchanged: route → rerank →
/// diversify → top-k through `pipeline::search`, then emit the memory
/// `Row` envelope. Split out of [`run`] to keep it under the function
/// length gate. `filters.scope` carries the time-scope both for the
/// pipeline call and the emitted `ScopeEcho`.
async fn run_memory(
    a: &Args,
    json_flag: bool,
    paths: &Paths,
    conn: &rusqlite::Connection,
    cfg: &crate::config::Config,
    filters: Filters<'_>,
    window: pipeline::PageWindow,
) -> Result<()> {
    let vec = embedding_input::read_optional(a.vector_stdin, a.vector.as_deref())?;
    let run = pipeline::search(
        cfg,
        conn,
        &a.query,
        vec.as_deref(),
        filters,
        pipeline::SearchOptions {
            track: track_searches()?,
            source: crate::stats::source::SEARCH,
            window,
        },
    )?;
    let meta = page_meta(window, run.has_more, run.total);
    // Batch-fetch navigation metadata (path/repo/kind/tags/references) for
    // this page's hits so each result is navigable without a per-hit query.
    let ids: Vec<&str> = run.hits.iter().map(|h| h.memory_id.as_str()).collect();
    let nav = memory_meta::fetch_meta(conn, &ids)?;
    output::search::emit(
        &run.hits,
        run.query_id.as_deref(),
        meta,
        json_flag,
        &nav,
        paths.data_dir(),
        output::search::ScopeEcho::of(filters.scope),
    )
}
