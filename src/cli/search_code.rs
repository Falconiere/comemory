//! `comemory search-code` — ranked search over the indexed `code_symbols`
//! table (BM25 + optional BYO-vector ANN, RRF-fused), reranked by the
//! PageRank / activation / working-set affinity / feedback priors.
//!
//! Mirrors `comemory search` (`crate::cli::search`): resolve the data dir,
//! open `comemory.db`, parse any caller-supplied vector, route via
//! [`crate::retrieval::code_route::route_code`], rerank via
//! [`crate::retrieval::code_rerank::rerank_code`], cut to `top_k`, record
//! telemetry, and emit. Code vectors are 768-dim (vs 1024 for memories);
//! the dim guard lives inside `store::vector::knn_code`, so a wrong-dim
//! vector fails there, not at parse time.
//!
//! ## Working-set affinity scope
//!
//! The affinity prior needs the repo's checkout path, which the database
//! does not know — `search-code` may run from anywhere. Decision: the
//! process CWD is used as the working-tree candidate (via the shared
//! [`WorkingSet::from_cwd`] policy, which also covers `context`), so the
//! affinity boost activates only when the command runs inside the
//! relevant repo's checkout (documented in `--help`). The detection is
//! best-effort by contract — a non-repo CWD degrades to the empty set
//! and a neutral prior.

use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::api::{self, Ctx};
use crate::cli::{embedding_input, lazy_reindex, load_config, resolve_data_dir, track_searches};
use crate::config::paths::Paths;
use crate::output;
use crate::prelude::*;
use crate::store::connection;

// The closing working-set caveat paragraph is intentionally duplicated in
// `cli::context::EXAMPLES` (same semantics; only the command name and the
// indexed/referenced adjective differ). clap's `after_help` plus the
// regenerated docs/cli-reference.md freeze the exact wrapped text, so a
// shared const cannot reproduce both renderings. A drift tripwire in
// `tests/cli/search_code.rs` asserts the two paragraphs stay equivalent.
const EXAMPLES: &str = "\
Examples:
  # Lexical code search; identifier tokens split automatically
  comemory search-code \"parse frontmatter\"

  # JSON output; hits[].score_parts breaks down every ranking factor
  # (relevance, rank, activation, affinity, feedback, final_score) and
  # the envelope carries query_id — pass it to
  # `comemory feedback <query_id> --used-code <ids>`.
  comemory search-code \"dim guard\" --json

  # Scope to one repo and language (aliases like `rs`/`py` accepted)
  comemory search-code \"router\" --repo myrepo --lang rust

  # Caller-supplied vector (BYO-vector; code vectors are 768-dim)
  comemory search-code \"knn\" --vector 0.1,0.2,0.3,...

The working-set affinity boost applies only when search-code runs inside
the indexed repo's checkout (the CWD is used to detect dirty/recent files)
AND the repo label used at index time (`index-code --repo`) matches the
--repo flag — or, when --repo is omitted, the checkout directory's
basename.";

/// Arguments to `comemory search-code`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Natural-language or identifier query string.
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
    /// Restrict hits to one repo label (as passed to `index-code --repo`).
    #[arg(long)]
    pub repo: Option<String>,
    /// Restrict hits to one language: `rust`, `typescript`, `javascript`,
    /// `python`, `go` (short aliases like `rs`/`ts`/`py` accepted).
    #[arg(long)]
    pub lang: Option<String>,
    /// Caller-supplied dense vector as a comma-separated float list.
    #[arg(long)]
    pub vector: Option<String>,
    /// Read a JSON `{ "embedding": [..] }` payload from stdin and use it as
    /// the dense vector for the query.
    #[arg(long, default_value_t = false)]
    pub vector_stdin: bool,
}

/// Run `comemory search-code`. Opens the DB, fires the lazy auto-reindex
/// trigger (a CLI-only affordance — see `api::search_code`'s doc),
/// resolves the vector input (if any), then delegates the shared middle to
/// `api::search_code::run` before emitting results in either TTY or JSON
/// form.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    // Validate `--lang` before any I/O — an unsupported value must fail
    // instantly with zero side effects (no db open, no lazy-reindex
    // trigger). `api::search_code::run` re-validates internally too
    // (defense-in-depth for the HTTP path).
    api::search_code::canonical_lang(a.lang.as_deref())?;
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut conn = connection::open(paths.db_path())?;
    let cfg = load_config(&paths)?;
    // Non-blocking lazy auto-reindex: under `auto_reindex = lazy`, fire a
    // detached `index-code` when this repo's HEAD has moved since the last
    // index, then search the current (possibly slightly stale) index. No-op
    // for `hook`/`off`, off-repo, or a fresh index. Never blocks or fails the
    // search (see `cli::lazy_reindex`).
    lazy_reindex::maybe_trigger(&conn, &cfg, &paths, a.repo.as_deref());

    let vector = embedding_input::read_optional(a.vector_stdin, a.vector.as_deref())?;
    let req = api::search_code::Request {
        query: a.query,
        k: a.k,
        offset: a.offset,
        repo: a.repo,
        lang: a.lang,
        vector,
    };
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let result = api::search_code::run(&mut ctx, req, track_searches()?)?;
    output::search_code::emit(
        &result.hits,
        result.query_id.as_deref(),
        result.meta,
        result.index_empty,
        json_flag,
    )
}
