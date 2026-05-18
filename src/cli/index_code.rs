//! `qwick-memory index-code` — walk a repo, extract symbols, embed snippets with
//! jina-code, and upsert into the LanceDB `code_chunks` table. Repo name is
//! auto-detected from the root path basename when `--repo` is omitted.
//!
//! The `--incremental` and `--quiet` flags are accepted in this task to keep
//! the CLI shape stable; Task 19 wires their actual semantics (skip rows with
//! unchanged `ast_hash`, suppress the human-readable line). For now,
//! `--quiet` suppresses TTY output and `--incremental` is a no-op pending
//! that follow-up.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::index::{CodeIndex, Embedder};
use crate::output::json;
use crate::prelude::*;

const EXAMPLES: &str = "\
Examples:
  # Index the current working directory
  qwick-memory index-code

  # Explicit root and repo label
  qwick-memory index-code --root /path/to/repo --repo qwick-backend

  # Incremental refresh, no human output
  qwick-memory index-code --incremental --quiet";

/// Arguments to `qwick-memory index-code`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Repo root to walk. Defaults to the current working directory.
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// Repo label stored in the `qualified` key. Auto-detected from `root`
    /// basename when empty.
    #[arg(long, default_value = "")]
    pub repo: String,
    /// Skip rows whose `ast_hash` is unchanged. Reserved for Task 19; accepted
    /// but currently a no-op.
    #[arg(long, default_value_t = false)]
    pub incremental: bool,
    /// Suppress the human-readable summary line. JSON output is still emitted
    /// when `--json` is set.
    #[arg(long, default_value_t = false)]
    pub quiet: bool,
}

/// JSON shape emitted under `--json`.
#[derive(Serialize)]
struct Output {
    repo: String,
    indexed_symbols: usize,
}

/// Walk + index the repo and report how many symbols landed in `code_chunks`.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let _ = a.incremental; // wired in Task 19; accepted now to keep CLI shape stable.
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let repo = if a.repo.is_empty() {
        detect_repo_name(&a.root)
    } else {
        a.repo.clone()
    };
    let idx = CodeIndex::open(paths.vectors_dir(), 768).await?;
    let mut emb = Embedder::jina_code()?;
    let n = idx.index_repo(&a.root, &repo, &mut emb).await?;

    let report = Output {
        repo: repo.clone(),
        indexed_symbols: n,
    };
    if json_flag {
        json::write(&report)?;
    } else if !a.quiet {
        let mut out = std::io::stdout().lock();
        writeln!(out, "indexed {n} symbols in repo '{repo}'")?;
    }
    Ok(())
}

/// Fall back to the canonicalized last path segment when callers omit
/// `--repo`. `"unknown"` is returned only when the path can't be resolved at
/// all (e.g. nonexistent root).
fn detect_repo_name(root: &Path) -> String {
    root.canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "unknown".into())
}
