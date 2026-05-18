//! `qwick search` — natural-language vector search over the memory index.
//! Returns top-K hits with score, repo, and a short body snippet.
//!
//! The retrieval pipeline lives in `crate::retrieval`: every query is first
//! classified into a [`Route`] (Hybrid / Symbol / FtsFirst), then run through
//! `search_memory` with a Config-default similarity threshold, and finally
//! checked against the corrective-fallback signal for observability (we log
//! when confidence is low but do not yet change behaviour — that lands when
//! the corrective second-pass is wired in).
//!
//! [`Route`]: crate::retrieval::Route

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::index::{Embedder, MemoryIndex};
use crate::prelude::*;
use crate::retrieval::corrective::should_fallback;
use crate::retrieval::hybrid::search_memory;
use crate::retrieval::{classify, Route};

/// Default similarity threshold for the memory layer. Mirrors the value the
/// design spec earmarks for the eventual `Config` surface; hardcoded here
/// until the config module is wired into the CLI.
const MEMORY_THRESHOLD: f32 = 0.55;

/// Minimum top1 / top2 score gap below which we log a "confidence low"
/// warning. Matches the corrective-fallback contract: see
/// `retrieval::corrective::should_fallback`.
const FALLBACK_MIN_CONFIDENCE: f32 = 0.15;

/// Arguments to `qwick search`.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Natural-language query string.
    pub query: String,
    /// Maximum number of hits to return (default 12).
    #[arg(long, default_value_t = 12)]
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

/// Embed the query, search the vector index, and render hits.
pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let idx = MemoryIndex::open(paths.vectors_dir(), 768).await?;
    let mut emb = Embedder::nomic_text()?;
    let q = emb.embed_one(&a.query)?;

    let route = classify(&a.query);
    let hits = search_memory(&idx, &q, a.limit, MEMORY_THRESHOLD).await?;

    // Observability only: if confidence is low and the result set is sparse,
    // surface it via `tracing` so operators can spot weak queries. The
    // pipeline keeps the original hits — second-pass corrective retrieval
    // is a follow-up task.
    if should_fallback(&hits, FALLBACK_MIN_CONFIDENCE) && hits.len() < a.limit {
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
