//! `qwick save` — write a new memory to disk via `MemoryStore::save`, then
//! best-effort wire the record into the kuzu property graph (Memory node +
//! provenance edges + cross-link references). Graph errors are logged and
//! swallowed because markdown remains the source of truth.

use std::io::Read;
use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::graph::cross_link::extract_refs;
use crate::graph::Graph;
use crate::memory::{Kind, MemoryStore};
use crate::prelude::*;

/// Arguments to `qwick save`. The positional `body` is optional — if omitted
/// or `-`, the body is read from stdin so callers can pipe content.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Memory body. Use `-` (or omit) to read from stdin.
    pub body: Option<String>,
    /// Memory kind: decision|bug|convention|discovery|pattern|note.
    #[arg(long, value_enum, default_value_t = Kind::Note)]
    pub kind: Kind,
    /// Repo name attached to the memory (free-form string).
    #[arg(long, default_value = "")]
    pub repo: String,
    /// Comma-separated tag list (e.g. `database,postgres`).
    #[arg(long, default_value = "")]
    pub tags: String,
    /// Author identifier. Defaults to empty so callers may omit.
    #[arg(long, default_value = "")]
    pub author: String,
    /// Quality rating 1..=5. Defaults to 3.
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u8).range(1..=5))]
    pub quality: u8,
}

/// JSON shape emitted under `--json`.
#[derive(Serialize)]
struct Output {
    id: String,
    path: String,
}

/// Save the body and emit the new memory id + on-disk path.
pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let body = match a.body.as_deref() {
        Some("-") | None => read_stdin()?,
        Some(s) => s.to_string(),
    };
    let tags: Vec<String> = if a.tags.is_empty() {
        Vec::new()
    } else {
        a.tags.split(',').map(|t| t.trim().to_string()).collect()
    };
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let store = MemoryStore::new(paths.clone());
    let rec = store.save(&body, a.kind, &a.repo, &tags, &a.author, a.quality)?;

    // Best-effort graph upsert: markdown is the source of truth, so any kuzu
    // failure is logged and swallowed rather than propagated to the user.
    if let Err(e) = upsert_graph(&paths, &rec) {
        tracing::warn!("graph upsert failed: {e}");
    }

    let output = Output {
        id: rec.frontmatter.id.clone(),
        path: rec.path.to_string_lossy().into_owned(),
    };
    let mut out = std::io::stdout().lock();
    if json {
        writeln!(out, "{}", serde_json::to_string(&output)?)?;
    } else {
        writeln!(out, "saved {}", output.id)?;
        writeln!(out, "  path: {}", output.path)?;
    }
    Ok(())
}

/// Open the graph, upsert the Memory + provenance edges, and add
/// `:ReferencesFile` / `:ReferencesSymbol` edges for every cross-link found in
/// the body. The `MATCH`-based reference edges silently no-op when their
/// target File/Symbol nodes do not yet exist, which is the intended
/// best-effort semantics: the code-layer indexer can fill them in later.
fn upsert_graph(paths: &Paths, rec: &crate::memory::MemoryRecord) -> Result<()> {
    let g = Graph::open(paths.graph_dir())?;
    g.upsert_memory(rec)?;
    let refs = extract_refs(&rec.body);
    for file_q in &refs.files {
        g.add_references_file(&rec.frontmatter.id, file_q)?;
    }
    for sym_q in &refs.symbols {
        g.add_references_symbol(&rec.frontmatter.id, sym_q)?;
    }
    Ok(())
}

fn read_stdin() -> Result<String> {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    Ok(buf)
}
