//! `comemory memory-for` — list memories that reference a qualified symbol or
//! file path. Filters `MemoryStore::list()` by frontmatter `references.symbols`
//! (exact match) and `references.files` (prefix match: the `qualified`
//! argument starts with the stored file path).
//!
//! Today the `references` block is populated by Task 14's extractor but the
//! save flow doesn't yet persist it (wired by a later task). Until then this
//! command commonly returns an empty list — the filter logic itself is what
//! we cover here.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::memory::MemoryStore;
use crate::output::json;
use crate::prelude::*;

const EXAMPLES: &str = "\
Examples:
  # Memories that reference a specific function
  comemory memory-for myrepo:src/db.rs:run_migration

  # Memories that reference a whole file
  comemory memory-for myrepo:src/db.rs

  # JSON for tool chaining
  comemory memory-for myrepo:src/db.rs --json";

/// Arguments to `comemory memory-for`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Qualified symbol (`<repo>:<path>:<symbol>`) or file path
    /// (`<repo>:<path>`) to look up.
    pub qualified: String,
}

/// One row of `comemory memory-for` output.
#[derive(Serialize)]
struct Row {
    id: String,
    repo: String,
    kind: String,
    snippet: String,
}

/// List memories whose frontmatter references `a.qualified`.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let store = MemoryStore::new(paths);
    let mems = store.list()?;
    let rows: Vec<Row> = mems
        .into_iter()
        .filter(|m| {
            m.frontmatter
                .references
                .symbols
                .iter()
                .any(|s| s == &a.qualified)
                || m.frontmatter
                    .references
                    .files
                    .iter()
                    .any(|f| a.qualified.starts_with(f))
        })
        .map(|m| Row {
            id: m.frontmatter.id.clone(),
            repo: m.frontmatter.repo.clone(),
            kind: format!("{:?}", m.frontmatter.kind).to_lowercase(),
            snippet: m.body.chars().take(160).collect(),
        })
        .collect();

    if json_flag {
        json::write(&rows)?;
    } else {
        let mut out = std::io::stdout().lock();
        for r in &rows {
            writeln!(out, "{} ({}) {}", r.id, r.kind, r.repo)?;
            writeln!(out, "  {}", r.snippet)?;
        }
    }
    Ok(())
}
