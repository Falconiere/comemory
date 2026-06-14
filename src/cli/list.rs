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

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::pagination::PaginationArgs;
use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::output::page::Page;
use crate::output::{json, tty};
use crate::prelude::*;
use crate::store::connection;
use crate::store::memory_list::{self, ListRow};

const EXAMPLES: &str = "\
Examples:
  # All decisions in a single repo
  comemory list --repo myrepo --kind decision

  # Every memory across all repos, JSON
  comemory list --json

  # Filter by kind only
  comemory list --kind bug

  # Second page of 20 memories
  comemory list --limit 20 --offset 20";

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
    /// `--limit` / `--offset` window over the listed memories.
    #[command(flatten)]
    pub page: PaginationArgs,
}

/// One row of `comemory list` output.
#[derive(Serialize)]
struct Row {
    id: String,
    kind: String,
    repo: String,
    slug: String,
}

impl From<ListRow> for Row {
    fn from(r: ListRow) -> Self {
        Self {
            id: r.id,
            kind: r.kind,
            repo: r.repo,
            slug: r.slug,
        }
    }
}

/// List filtered memories from the SQLite mirror as a paginated `Page<Row>`.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let conn = connection::open(paths.db_path())?;

    // Kind is matched case-insensitively against the canonical lowercase
    // `memories.kind` values, mirroring the legacy `eq_ignore_ascii_case`
    // filter so `--kind Decision` and `--kind decision` behave identically.
    let kind = a.kind.as_deref().map(str::to_ascii_lowercase);
    let listed = memory_list::list_memories(
        &conn,
        a.repo.as_deref(),
        kind.as_deref(),
        a.page.limit,
        a.page.offset,
    )?;

    let rows: Vec<Row> = listed.rows.into_iter().map(Row::from).collect();
    let offset = a.page.offset;
    let has_more = offset + rows.len() < listed.total;
    let page = Page::new(rows, a.page.limit, offset, Some(listed.total), has_more);

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
