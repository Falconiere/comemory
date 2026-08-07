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

use crate::api::{self, Ctx};
use crate::cli::{embedding_input, lazy_reindex, load_config, track_searches};
use crate::config::paths::{Paths, resolve_data_dir};
use crate::output;
use crate::prelude::*;
use crate::store::connection;

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
    /// Only consider memories created at or after this instant. Accepts an
    /// RFC3339 timestamp or a bare `YYYY-MM-DD` date (start of that UTC day).
    #[arg(long, value_name = "WHEN")]
    pub since: Option<String>,
    /// Only consider memories created at or before this instant. Accepts an
    /// RFC3339 timestamp or a bare `YYYY-MM-DD` date (end of that UTC day).
    /// Filters candidates only — the supersede penalty stays present-day.
    #[arg(long, value_name = "WHEN")]
    pub until: Option<String>,
    /// Bundle the corpus as it stood at this instant: `--until` plus
    /// supersede-penalty scoping, so a memory counts as superseded only by
    /// one that already existed then. Same value grammar as `--until`.
    #[arg(long = "as-of", value_name = "WHEN", conflicts_with = "until")]
    pub as_of: Option<String>,
}

/// Run `comemory context`. Opens the DB, fires the lazy auto-reindex trigger
/// (a CLI-only affordance — see `api::context`'s doc), then delegates the
/// shared middle to `api::context::run`. The lookup is tracked like a
/// search, and the resulting `query_id` is surfaced (JSON field / TTY
/// footer) so context lookups can receive `comemory feedback` instead of
/// polluting reformulation mining as permanently-failed queries.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut conn = connection::open(paths.db_path())?;
    let cfg = load_config(&paths)?;
    // Non-blocking lazy auto-reindex (see `cli::lazy_reindex`): fire a
    // detached `index-code` when this repo's HEAD moved since the last index,
    // then proceed against the current index. No-op for `hook`/`off`,
    // off-repo, or a fresh index; never blocks or fails the lookup.
    lazy_reindex::maybe_trigger(&conn, &cfg, &paths, a.repo.as_deref());

    let vector = embedding_input::read_optional(a.vector_stdin, a.vector.as_deref())?;
    let req = api::context::Request {
        query: a.query,
        k: a.k,
        offset: a.offset,
        repo: a.repo,
        vector,
        since: a.since,
        until: a.until,
        as_of: a.as_of,
    };
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let result = api::context::run(&mut ctx, req, track_searches()?)?;
    output::context::emit(&result, json_flag)
}
