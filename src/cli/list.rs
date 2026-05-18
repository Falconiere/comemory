//! `qwick-memory list` — enumerate memories on disk with optional `--repo` / `--kind`
//! filters. Output is one row per memory in TTY mode or a JSON array under
//! `--json`.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::memory::MemoryStore;
use crate::output::json;
use crate::prelude::*;

/// Arguments to `qwick-memory list`.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Filter to memories whose `repo` matches exactly.
    #[arg(long)]
    pub repo: Option<String>,
    /// Filter by kind (case-insensitive): decision|bug|convention|discovery|pattern|note.
    #[arg(long)]
    pub kind: Option<String>,
}

/// One row of `qwick-memory list` output.
#[derive(Serialize)]
struct Row {
    id: String,
    kind: String,
    repo: String,
    slug: String,
}

/// List filtered memories from disk.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut all = MemoryStore::new(paths).list()?;
    if let Some(r) = a.repo {
        all.retain(|m| m.frontmatter.repo == r);
    }
    if let Some(k) = a.kind {
        all.retain(|m| format!("{:?}", m.frontmatter.kind).eq_ignore_ascii_case(&k));
    }

    let rows: Vec<Row> = all
        .into_iter()
        .map(|m| Row {
            id: m.frontmatter.id.clone(),
            kind: format!("{:?}", m.frontmatter.kind).to_lowercase(),
            repo: m.frontmatter.repo.clone(),
            slug: m
                .path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
        })
        .collect();

    if json_flag {
        json::write(&rows)?;
    } else {
        let mut out = std::io::stdout().lock();
        for r in &rows {
            writeln!(out, "{}  {}  {}  {}", r.id, r.kind, r.repo, r.slug)?;
        }
    }
    Ok(())
}
