//! `comemory list` — enumerate memories with optional `--repo` / `--kind`
//! filters and a `--limit` / `--offset` window.
//!
//! Source change: `list` now reflects the `comemory.db` SQLite mirror (kept in
//! sync on every `comemory save`; reconstructable from `memories/*.md` via
//! `comemory rebuild`), not a live markdown scan. Filters and the window are
//! pushed into SQL so cost scales with the page, not the corpus. Output is the
//! shared `Page<Row>` envelope under `--json` (was a bare array) and one row
//! per memory plus a pagination footer in TTY mode. The per-item `Row` fields
//! (`id`, `kind`, `repo`, `slug`) are unchanged.

use std::io::Write as _;
use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};

use crate::api::{self, Ctx};
use crate::cli::load_config;
use crate::cli::pagination::PaginationArgs;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::output::{json, tty};
use crate::prelude::*;
use crate::store::connection;

const EXAMPLES: &str = "\
Examples:
  # All decisions in a single repo
  comemory list --repo myrepo --kind decision

  # Every memory across all repos, JSON
  comemory list --json

  # Filter by kind only
  comemory list --kind bug

  # Tagged `postgres`, quality 4+, mentioning \"pool\" anywhere in the body
  comemory list --tag postgres --min-quality 4 --query pool

  # Second page of 20 memories
  comemory list --limit 20 --offset 20";

/// Sort order for `comemory list`'s rows.
#[derive(Copy, Clone, Debug, ValueEnum)]
#[clap(rename_all = "lowercase")]
pub enum Sort {
    /// Newest created first (default).
    Created,
    /// Descending quality.
    Quality,
    /// Most-recently-accessed first; never-accessed rows sort last.
    Accessed,
}

/// Arguments to `comemory list`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Filter to memories whose `repo` matches exactly.
    #[arg(long)]
    pub repo: Option<String>,
    /// Filter by kind (case-insensitive): decision|bug|convention|discovery|pattern|note.
    #[arg(long)]
    pub kind: Option<String>,
    /// Filter to memories carrying this exact tag.
    #[arg(long)]
    pub tag: Option<String>,
    /// Filter to memories whose quality is at least this (1..=5).
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=5))]
    pub min_quality: Option<u8>,
    /// Filter to memories whose body contains this text (case-insensitive,
    /// matched literally).
    #[arg(long)]
    pub query: Option<String>,
    /// Sort order: `created` (default, newest first) | `quality`
    /// (descending) | `accessed` (most-recently-accessed first).
    #[arg(long, value_enum, default_value_t = Sort::Created)]
    pub sort: Sort,
    /// `--limit` / `--offset` window over the listed memories.
    #[command(flatten)]
    pub page: PaginationArgs,
}

/// List filtered memories from the SQLite mirror as a paginated `Page<Row>`.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut conn = connection::open(paths.db_path())?;
    let cfg = load_config(&paths)?;

    let req = api::list::Request {
        repo: a.repo,
        kind: a.kind,
        tag: a.tag,
        min_quality: a.min_quality,
        q: a.query,
        limit: a.page.limit,
        offset: a.page.offset,
        sort: match a.sort {
            Sort::Created => api::list::Sort::Created,
            Sort::Quality => api::list::Sort::Quality,
            Sort::Accessed => api::list::Sort::Accessed,
        },
    };
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let page = api::list::run(&mut ctx, req)?;

    if json_flag {
        json::write(&page)?;
    } else {
        let mut out = std::io::stdout().lock();
        for r in &page.items {
            writeln!(out, "{}  {}  {}  {}", r.id, r.kind, r.repo, r.slug)?;
        }
        tty::write_page_footer(&mut out, page.items.len(), page.offset, page.total)?;
    }
    Ok(())
}
