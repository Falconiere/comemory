//! `qwick-memory symbol` — semantic search over the code index. Embeds the query
//! name with jina-code, queries the `code_chunks` table for the top hits,
//! and renders qualified name + similarity score + a short snippet preview.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::index::{CodeIndex, Embedder};
use crate::output::json;
use crate::prelude::*;
use crate::retrieval::hybrid::search_code;

const EXAMPLES: &str = "\
Examples:
  # Exact function-name hit
  qwick-memory symbol run_migration

  # Natural-language descriptor, top 10 JSON
  qwick-memory symbol \"parse frontmatter yaml\" --limit 10 --json

  # Broader semantic match
  qwick-memory symbol \"embed query string into vector\"";

/// Arguments to `qwick-memory symbol`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Free-form symbol name (or descriptor) to search for.
    pub name: String,
    /// Maximum number of hits to return (default 5).
    #[arg(long, default_value_t = 5)]
    pub limit: usize,
}

/// One row of `qwick-memory symbol` output.
#[derive(Serialize)]
struct Row {
    qualified: String,
    score: f32,
    snippet: String,
}

/// Embed the query name, search `code_chunks`, and render top hits.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let idx = CodeIndex::open(paths.vectors_dir(), 768).await?;
    let mut emb = Embedder::jina_code()?;
    let q = emb.embed_one(&a.name)?;
    let hits = search_code(&idx, &q, a.limit, 0.0).await?;
    let rows: Vec<Row> = hits
        .into_iter()
        .map(|h| Row {
            qualified: h.qualified,
            score: h.score,
            snippet: h.snippet.chars().take(200).collect(),
        })
        .collect();

    if json_flag {
        json::write(&rows)?;
    } else {
        let mut out = std::io::stdout().lock();
        for r in &rows {
            writeln!(out, "{:.3}  {}", r.score, r.qualified)?;
            writeln!(out, "  {}", r.snippet)?;
        }
    }
    Ok(())
}
