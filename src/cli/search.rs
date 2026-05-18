//! `qwick search` — natural-language vector search over the memory index.
//! Returns top-K hits with score, repo, and a short body snippet.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::index::{Embedder, MemoryIndex};
use crate::prelude::*;
use crate::retrieval::hybrid::search_memory;

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

/// Embed the query, search the vector index, and render hits.
pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let idx = MemoryIndex::open(paths.vectors_dir(), 768).await?;
    let mut emb = Embedder::nomic_text()?;
    let q = emb.embed_one(&a.query)?;
    let hits = search_memory(&idx, &q, a.limit, 0.0).await?;
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
        writeln!(out, "{}", serde_json::to_string(&rows)?)?;
    } else {
        for r in &rows {
            writeln!(out, "{:.3}  {}  {}  {}", r.score, r.id, r.repo, r.snippet)?;
        }
    }
    Ok(())
}
