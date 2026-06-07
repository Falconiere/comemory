//! `comemory save` — write a new memory to disk via `MemoryStore::save`, then
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

const EXAMPLES: &str = "\
Examples:
  # Save a decision with tags and elevated quality
  comemory save \"Use Postgres for analytics\" --kind decision --repo myrepo --tags db,postgres --quality 4

  # Pipe a bug report body from another command
  echo \"Race in run_migration when run twice in <1s\" | comemory save - --kind bug --repo myrepo

  # Read the body from a file via shell redirect
  comemory save - --kind discovery --repo myrepo < notes/postgres-migration.md

  # Minimal note (kind defaults to `note`, no repo/tags)
  comemory save \"Remember: cargo nextest serializes the embedder group\"

  # Batch import: skip the per-save embedder load, then rebuild indices once
  for f in *.md; do comemory save - --no-index < \"$f\"; done && comemory index-code";

/// Arguments to `comemory save`. The positional `body` is optional — if omitted
/// or `-`, the body is read from stdin so callers can pipe content.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
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
    /// Skip the dense embed + FTS upsert (markdown + graph still run). Use
    /// for batch imports; rebuild the dense table afterwards with
    /// `comemory index-code` (or a future dedicated `comemory index` command).
    #[arg(long, default_value_t = false)]
    pub no_index: bool,
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

    // Best-effort dense embedding + FTS upsert. Either failure logs and is
    // swallowed: markdown remains the source of truth, and the user-facing
    // `comemory save` path must not fail just because LanceDB or SQLite
    // cannot open under the current data dir.
    //
    // `--no-index` skips this step entirely so batch / scripted saves do not
    // pay the ~300ms–2s cold-load cost of `Embedder::nomic_text()` per file.
    if !a.no_index {
        if let Err(e) = upsert_indices(&paths, &rec).await {
            tracing::warn!("index upsert failed: {e}");
            // Best-effort durable signal: record the failure in the stats DB
            // so `comemory doctor` (and operators tailing the stats file)
            // can see an indexing problem instead of just a vanished warn!.
            // A stats-DB failure itself is logged and ignored — markdown is
            // still the source of truth.
            if let Err(stats_err) = record_index_failure(&paths, &e.to_string()) {
                tracing::warn!("record_index_failure: {stats_err}");
            }
        }
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

/// Best-effort dense + lexical index writes for the freshly saved memory.
/// Opens the LanceDB `memory_chunks` table, embeds the body with the
/// nomic-text model, upserts the row, then mirrors the same id+body into
/// the SQLite FTS5 table. Called from `run` under a `warn!`-and-swallow
/// guard so embedder or filesystem hiccups never block a save: markdown
/// remains the source of truth and `comemory index-code` (or a future
/// dedicated `comemory index` command) can rebuild what was missed.
async fn upsert_indices(paths: &Paths, rec: &crate::memory::MemoryRecord) -> Result<()> {
    let idx = crate::index::MemoryIndex::open(paths.vectors_dir(), 768).await?;
    let mut emb = crate::index::Embedder::nomic_text()?;
    let v = emb.embed_one(&rec.body)?;
    idx.upsert(rec, &v).await?;

    let fts = crate::index::Fts::open(paths.index_dir().join("fts.sqlite"))?;
    fts.upsert(&rec.frontmatter.id, &rec.body)?;
    Ok(())
}

/// Open the stats SQLite database and append one row to `index_failures`.
/// Used by [`run`] when `upsert_indices` fails: callers swallow the
/// indexing error but we still want a durable counter so `comemory doctor`
/// (and operators) can spot a broken indexing pipeline. The stats DB
/// itself is best-effort — a write failure is logged by the caller.
fn record_index_failure(paths: &Paths, error: &str) -> Result<()> {
    let db = crate::stats::StatsDb::open(paths.stats_db())?;
    db.record_index_failure(time::OffsetDateTime::now_utc(), error)
}
