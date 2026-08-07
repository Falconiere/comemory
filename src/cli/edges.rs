//! `comemory edges` — search the relation graph lexically.
//!
//! The fourth free-text surface, alongside `search`, `search-code`, and
//! `context`. It reads the `edge_fts` triplet index
//! ([`crate::store::edge_fts`]), where every `edges` row is rendered as
//! searchable `src —rel→ dst` text, so questions the graph could always
//! answer by id ("what supersedes the queue decision?", "what imports the
//! parser?") become answerable by words.
//!
//! Ranking is plain BM25 with a deterministic `(src_id, rel, dst_id)`
//! tie-break — no rerank, no priors, no vectors. Results never mix into
//! `search` / `search-code` output.

use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::api::{self, Ctx};
use crate::cli::load_config;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::output;
use crate::prelude::*;
use crate::store::connection;

const EXAMPLES: &str = "\
Examples:
  # Which memory supersedes the queue decision?
  comemory edges \"supersedes queue design\"

  # Machine-readable rows plus the page cursor
  comemory edges \"imports tokenizer\" --json

  # Second page of ten relations
  comemory edges tokenizer --k 10 --offset 10

The query is matched against the rendered triplet text: a memory endpoint
renders as `<kind> <slug>`, a file or symbol endpoint as its id with the
kind prefix stripped. Two tiers are tried — strict AND, then word-OR — and
the tier is chosen once per query, so paging never switches ladders midway.";

/// Arguments to `comemory edges`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Free-form query — relation verb, memory slug words, path or symbol
    /// fragment, or any mix of them.
    pub query: String,
    /// Page size — overrides the configured `retrieval.top_k`. `--limit` is
    /// an accepted alias. `0` means "all remaining within the
    /// `max_page_window`".
    #[arg(long, visible_alias = "limit")]
    pub k: Option<usize>,
    /// Number of leading ranked relations to skip (deep paging).
    #[arg(long, default_value_t = 0)]
    pub offset: usize,
}

/// Run `comemory edges`: open the store, then delegate the shared middle
/// (self-heal + triplet paging) to `api::edges::run`.
///
/// Migration 0012 creates `edge_fts` empty on purpose — the rendering lives
/// in Rust and only there — so a database upgraded from an earlier version
/// arrives with edges but no triplets. The CLI always allows the self-heal
/// (`allow_self_heal: true`), so the first query materializes them, healing
/// the upgrade in the same run with no flag and no migration backfill. A
/// refresh failure is propagated rather than swallowed: answering an empty
/// page from a stale index would be a lie, not a degradation.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut conn = connection::open(paths.db_path())?;
    let cfg = load_config(&paths)?;

    let req = api::edges::Request {
        query: a.query,
        k: a.k,
        offset: a.offset,
    };
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let result = api::edges::run(&mut ctx, req, true)?;
    output::edges::emit(
        &result.hits,
        result.limit,
        result.offset,
        result.has_more,
        json_flag,
    )
}
