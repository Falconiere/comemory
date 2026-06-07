//! `comemory context` — headline lookup. Embeds `key` with **both** the
//! jina-code embedder (for the `code_chunks` table) and the nomic-text
//! embedder (for the memory index), then returns a single `ContextBundle`
//! with the best matching code symbol + top memories. JSON output is
//! machine-friendly; TTY output reads as "here's the symbol, here are the
//! memories".
//!
//! The `depth` flag is accepted for shape compatibility with Task 17's graph
//! walk extension; the current implementation surfaces the 0-hop slice
//! (direct vector hits) only.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::index::{CodeIndex, Embedder, MemoryIndex};
use crate::output::json;
use crate::prelude::*;
use crate::retrieval::hybrid::{search_code, search_memory};

const EXAMPLES: &str = "\
Examples:
  # Code symbol + linked memories in one round-trip (JSON)
  comemory context run_migration --json

  # Natural-language key with a deeper neighborhood walk
  comemory context \"postgres migration race\" --depth 2

  # File-path fragment as the key
  comemory context \"src/db.rs\"";

/// Arguments to `comemory context`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Free-form key — symbol name, file path fragment, or natural-language
    /// phrase. Embedded against both the code index and the memory index.
    pub key: String,
    /// Graph-walk depth. Reserved for Task 17 (Supersedes / ConflictsWith
    /// walks); accepted now to keep the CLI shape stable.
    #[arg(long, default_value_t = 1)]
    pub depth: u32,
}

/// JSON envelope returned to callers. Named `ContextBundle` so it doesn't
/// collide with `retrieval::Bundle`.
#[derive(Serialize)]
struct ContextBundle {
    key: String,
    symbol: Option<SymbolView>,
    memories: Vec<MemoryView>,
}

/// Top code-layer hit (or `None` when the code index is empty / has no hit).
#[derive(Serialize)]
struct SymbolView {
    qualified: String,
    snippet: String,
    score: f32,
}

/// One memory-layer hit. Score is kept so callers can rank-mix client-side.
#[derive(Serialize)]
struct MemoryView {
    id: String,
    kind: String,
    snippet: String,
    score: f32,
}

/// Build the bundle and render JSON or TTY view.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let _ = a.depth; // wired in Task 17 (graph walks).
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;

    let cidx = CodeIndex::open(paths.vectors_dir(), 768).await?;
    let mut code_emb = Embedder::jina_code()?;
    let code_q = code_emb.embed_one(&a.key)?;
    let code_hits = search_code(&cidx, &code_q, 1, 0.0).await?;
    let symbol = code_hits.into_iter().next().map(|h| SymbolView {
        qualified: h.qualified,
        snippet: h.snippet,
        score: h.score,
    });

    let midx = MemoryIndex::open(paths.vectors_dir(), 768).await?;
    let mut text_emb = Embedder::nomic_text()?;
    let text_q = text_emb.embed_one(&a.key)?;
    let mhits = search_memory(&midx, &text_q, 5, 0.0).await?;
    let memories: Vec<MemoryView> = mhits
        .into_iter()
        .map(|h| MemoryView {
            id: h.id,
            kind: format!("{:?}", h.kind).to_lowercase(),
            snippet: h.body.chars().take(200).collect(),
            score: h.score,
        })
        .collect();

    let bundle = ContextBundle {
        key: a.key.clone(),
        symbol,
        memories,
    };

    if json_flag {
        json::write(&bundle)?;
    } else {
        let mut out = std::io::stdout().lock();
        if let Some(s) = &bundle.symbol {
            writeln!(out, "symbol: {} ({:.3})", s.qualified, s.score)?;
            writeln!(out, "{}", s.snippet)?;
        }
        writeln!(out, "\n— memories —")?;
        for m in &bundle.memories {
            writeln!(out, "{:.3}  {} ({})", m.score, m.id, m.kind)?;
            writeln!(out, "  {}", m.snippet)?;
        }
    }
    Ok(())
}
