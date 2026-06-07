//! `comemory search` — natural-language search over the memory index.
//! Returns top-K hits with score, repo, and a short body snippet.
//!
//! The retrieval pipeline lives in `crate::retrieval`: every query is first
//! classified into a [`Route`] (Hybrid / Symbol / FtsFirst), then run through
//! `search_memory_fused` (RRF over dense + BM25), and finally checked against
//! the corrective-fallback signal for observability (we log when confidence is
//! low but do not yet change behaviour — that lands when the corrective
//! second-pass is wired in).
//!
//! [`Route`]: crate::retrieval::Route

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::file::Config;
use crate::config::paths::Paths;
use crate::index::{Embedder, MemoryIndex};
use crate::prelude::*;
use crate::retrieval::corrective::should_fallback;
use crate::retrieval::fuse::search_memory_fused;
use crate::retrieval::{classify, Route};

const EXAMPLES: &str = "\
Examples:
  # Natural-language query, top 12 hits (default)
  comemory search \"postgres migration race\"

  # Limit hits and emit JSON for agent consumption
  comemory search \"what database do we use\" --limit 5 --json

  # Tightly scoped query
  comemory search \"tree-sitter ast pattern\" --limit 3";

/// Arguments to `comemory search`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Natural-language query string.
    pub query: String,
    /// Maximum number of hits to return (default 12). Must be >= 1.
    #[arg(
        long,
        default_value_t = 12,
        value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..)
    )]
    pub limit: usize,
}

/// One row of search output.
#[derive(Serialize)]
struct Row {
    id: String,
    score: f32,
    repo: String,
    snippet: String,
}

/// Envelope emitted under `--json`. Includes the route the classifier picked
/// so callers (and tests) can observe which retrieval branch fired without
/// re-implementing `classify`.
#[derive(Serialize)]
struct Envelope<'a> {
    route: &'static str,
    hits: &'a [Row],
}

/// Embed the query, run RRF-fused dense+BM25 retrieval, and render hits.
pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let cfg = Config::defaults().with_env()?;
    let idx = MemoryIndex::open(paths.vectors_dir(), 768).await?;
    let mut emb = Embedder::nomic_text()?;
    let q = emb.embed_one(&a.query)?;

    let route = classify(&a.query);
    let hits =
        search_memory_fused(&idx, &paths, &q, &a.query, a.limit, cfg.retrieval.rrf_k).await?;

    // Observability only: if confidence is low and the result set is sparse,
    // surface it via `tracing` so operators can spot weak queries. The
    // pipeline keeps the original hits — second-pass corrective retrieval
    // is a follow-up task.
    if should_fallback(&hits, cfg.retrieval.corrective_min_confidence) && hits.len() < a.limit {
        tracing::warn!("search confidence low: route={:?}", route);
    }

    let rows: Vec<Row> = hits
        .into_iter()
        .map(|h| Row {
            id: h.id,
            score: h.score,
            repo: h.repo,
            snippet: h.body.chars().take(160).collect(),
        })
        .collect();
    let mut out = std::io::stdout().lock();
    if json {
        let envelope = Envelope {
            route: route_label(route),
            hits: &rows,
        };
        writeln!(out, "{}", serde_json::to_string(&envelope)?)?;
    } else {
        for r in &rows {
            writeln!(out, "{:.3}  {}  {}  {}", r.score, r.id, r.repo, r.snippet)?;
        }
    }
    Ok(())
}

/// Map a [`Route`] to the JSON tag emitted under `--json`. Matches the variant
/// names so callers can pattern-match without an extra translation table.
fn route_label(route: Route) -> &'static str {
    match route {
        Route::Hybrid => "Hybrid",
        Route::Symbol => "Symbol",
        Route::FtsFirst => "FtsFirst",
    }
}
