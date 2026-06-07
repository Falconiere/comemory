//! `comemory index` — memory-layer index maintenance. The headline operation
//! is `--rebuild`: walk every markdown record under `~/.comemory/memories`,
//! diff against the LanceDB `memory_chunks` table, and re-embed any id that
//! is on disk but missing from the dense index.
//!
//! Useful after batch imports with `comemory save --no-index`, after a fresh
//! checkout where the LanceDB directory is gone, or to recover from an
//! interrupted save flow that left markdown but no dense row.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::index::{Embedder, MemoryIndex};
use crate::memory::MemoryStore;
use crate::output::json;
use crate::prelude::*;

/// Example invocations shown at the bottom of `comemory index --help`.
pub const EXAMPLES: &str = "\
Examples:
  # Re-embed every markdown memory missing from the dense index
  comemory index --rebuild

  # JSON summary for monitoring / CI
  comemory index --rebuild --json

  # Quiet rebuild (suppresses the human summary; JSON still respected)
  comemory index --rebuild --quiet";

/// Arguments to `comemory index`. Today only `--rebuild` is wired; future
/// operations (compact, prune-orphans) hang off the same subcommand.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Re-embed any markdown memory whose id is missing from the dense
    /// `memory_chunks` table.
    #[arg(long, default_value_t = false)]
    pub rebuild: bool,
    /// Suppress the human-readable summary line. JSON output is still
    /// emitted when `--json` is set.
    #[arg(long, default_value_t = false)]
    pub quiet: bool,
}

/// JSON shape emitted under `--json`.
#[derive(Serialize)]
struct Output {
    rebuilt: usize,
    total_memories: usize,
    total_indexed_before: usize,
}

/// Dispatch the `index` operation. Currently `--rebuild` is the only mode;
/// callers that omit it receive a usage error so we don't silently no-op.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    if !a.rebuild {
        return Err(Error::Other(
            "comemory index requires --rebuild (no other modes wired yet)".into(),
        ));
    }
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;

    let store = MemoryStore::new(paths.clone());
    let on_disk = store.list()?;
    let idx = MemoryIndex::open(paths.vectors_dir(), 768).await?;
    let indexed = idx.list_ids().await?;
    let indexed_set: std::collections::HashSet<&str> = indexed.iter().map(|s| s.as_str()).collect();

    let missing: Vec<_> = on_disk
        .iter()
        .filter(|m| !indexed_set.contains(m.frontmatter.id.as_str()))
        .collect();

    let mut emb = if missing.is_empty() {
        None
    } else {
        Some(Embedder::nomic_text()?)
    };

    let mut rebuilt = 0usize;
    for rec in &missing {
        // SAFETY-equivalent: `emb` is Some when `missing` is non-empty by
        // construction (see init above).
        let embedder = match emb.as_mut() {
            Some(e) => e,
            None => break,
        };
        let v = embedder.embed_one(&rec.body)?;
        idx.upsert(rec, &v).await?;
        rebuilt += 1;
    }

    let report = Output {
        rebuilt,
        total_memories: on_disk.len(),
        total_indexed_before: indexed.len(),
    };
    if json_flag {
        json::write(&report)?;
    } else if !a.quiet {
        let mut out = std::io::stdout().lock();
        writeln!(
            out,
            "rebuilt {} of {} memories (was {} indexed)",
            report.rebuilt, report.total_memories, report.total_indexed_before
        )?;
    }
    Ok(())
}
