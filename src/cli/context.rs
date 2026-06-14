//! `comemory context` — headline lookup over the v0.2 SQLite store.
//!
//! Runs the query through [`crate::retrieval::pipeline::search`] (the same
//! route → rerank → diversify path as `comemory search`) to surface
//! relevant memory ids, then assembles a [`crate::retrieval::bundle`] that
//! pulls each memory's body and any cross-link edges
//! (`references_file`, `references_symbol`, `relates_to`, `supersedes`)
//! up to depth 2. Code refs inside the bundle are ranked by the
//! [`crate::retrieval::code_prior`] product, with the working set built
//! from the process CWD via the shared [`WorkingSet::from_cwd`] policy
//! (same caveat as `search-code`: the affinity boost only activates
//! inside the referenced repo's checkout).

use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::{embedding_input, load_config, page_meta, page_window, resolve_data_dir};
use crate::config::paths::Paths;
use crate::output;
use crate::prelude::*;
use crate::retrieval::code_rerank::WorkingSet;
use crate::retrieval::{bundle, pipeline};
use crate::store::{code_row, connection};

// The closing working-set caveat sentence is intentionally duplicated in
// `cli::search_code::EXAMPLES` (same semantics; only the command name and
// the indexed/referenced adjective differ). clap's `after_help` plus the
// regenerated docs/cli-reference.md freeze the exact wrapped text, so a
// shared const cannot reproduce both renderings. A drift tripwire in
// `tests/cli/search_code.rs` asserts the two paragraphs stay equivalent.
const EXAMPLES: &str = "\
Examples:
  # Headline lookup for a symbol name, JSON envelope
  comemory context run_migration --json

  # Pin the bundle width to the top 3 hits
  comemory context \"advisory lock\" --k 3

  # ANN-assisted context with a caller-supplied vector
  comemory context \"advisory lock\" --vector 0.1,0.2,...

Code refs in the bundle are ranked by graph priors (PageRank, recency,
working-set affinity, feedback); each resolved ref carries a rank_parts
breakdown in --json mode. The working-set affinity boost applies only
when context runs inside the referenced repo's checkout (the CWD is used
to detect dirty/recent files) AND the repo label used at index time
(`index-code --repo`) matches the --repo flag — or, when --repo is
omitted, the checkout directory's basename.";

/// Arguments to `comemory context`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Free-form query — symbol name, file path fragment, or phrase.
    pub query: String,
    /// Page size for the bundle's memory list — overrides the configured
    /// `retrieval.top_k`. `--limit` is an accepted alias. `0` means "all
    /// remaining within the `max_page_window`".
    #[arg(long, visible_alias = "limit")]
    pub k: Option<usize>,
    /// Number of leading ranked memories to skip (deep paging of the
    /// bundle's memory list). Bounded by `retrieval.max_page_window`. Per-
    /// memory code refs are not paginated — each surfaced memory keeps its
    /// full ref set.
    #[arg(long, default_value_t = 0)]
    pub offset: usize,
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
/// cross-link edges walked to depth ≤ 2. The lookup is tracked like a
/// search, and the resulting `query_id` is surfaced (JSON field / TTY
/// footer) so context lookups can receive `comemory feedback` instead of
/// polluting reformulation mining as permanently-failed queries.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let conn = connection::open(paths.db_path())?;

    let vec = embedding_input::read_optional(a.vector_stdin, a.vector.as_deref())?;
    let cfg = load_config(&paths)?;
    let window = page_window(&cfg, a.k, a.offset);
    // A user-facing lookup always tracks. The flag is carried on
    // `SearchOptions` so the memory access bump (inside `pipeline::search`)
    // and the code-ref bump below share one gate: an eval/tune caller that
    // ever runs `context` with `track = false` suppresses both signals.
    let opts = pipeline::SearchOptions {
        track: true,
        source: crate::stats::source::CONTEXT,
        window,
    };
    let run = pipeline::search(
        &cfg,
        &conn,
        &a.query,
        vec.as_deref(),
        a.repo.as_deref(),
        None,
        opts,
    )?;
    let meta = page_meta(window, run.has_more, run.total);
    let query_id = run.query_id.clone();
    let ids: Vec<String> = run.hits.into_iter().map(|h| h.memory_id).collect();
    // Zero hits → no edges to walk, hence no code refs for the affinity
    // prior to boost, so the git discovery + status walk behind
    // `WorkingSet::from_cwd` is skipped (mirrors the `search-code` guard).
    let ws = if ids.is_empty() {
        WorkingSet::default()
    } else {
        WorkingSet::from_cwd(a.repo.as_deref())
    };
    let bundle = bundle::assemble(&conn, &cfg, &a.query, &ids, &ws)?;
    // Self-reinforce the code refs the bundle actually surfaced (resolved
    // to an indexed `code_symbols` row), the code-side twin of the memory
    // access bump `pipeline::search` already applied — gated by the same
    // `opts.track` flag and best-effort via the shared writer.
    if opts.track {
        code_row::record_access(&conn, &bundle.resolved_code_ids);
    }
    output::context::emit(&bundle, query_id.as_deref(), meta, json_flag)
}
