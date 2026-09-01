//! `comemory prune` — surface candidates for deletion against the SQLite mirror.
//!
//! Reported classes: orphan `edges` (source memory missing/deleted), stale code
//! files (`code_symbols` paths gone from `indexed_files`), low-value memories
//! (cold/unloved/low-quality/unreferenced or superseded), and ghost code-refs
//! (memories whose pinned symbol no longer resolves — advisory only, never
//! auto-deleted, per spec Non-Goal 5). The scan + apply middle lives in
//! `api::prune` (Binding Rule 1).
//!
//! Default is a dry run. `--apply` soft-deletes low-value memories through the
//! `comemory delete` path then runs the orphan/stale cleanup in one
//! transaction; ghost-ref candidates are surfaced but not deleted.

use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::api::{self, Ctx};
use crate::cli::load_config;
use crate::cli::pagination::PaginationArgs;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::output::prune as output;
use crate::prelude::*;
use crate::store::connection;

/// Example invocations shown at the bottom of `comemory prune --help`.
pub const EXAMPLES: &str = "\
Examples:
  # Default is a dry run: inspect candidates without mutating anything
  comemory prune

  # Apply: soft-delete low-value memories (markdown -> memories/.trash/)
  # and clean up orphan edges + stale code symbols
  comemory prune --apply

  # Page the dry-run lists (window applies to display only; --apply is
  # always full-set): second page of 20 candidates
  comemory prune --limit 20 --offset 20

  # JSON output for CI/automation; Report fields:
  #   low_value_memories / stale_code_files / ghost_ref_memories — Page
  #     envelopes ({items, limit, offset, total, has_more}). low_value ids
  #     match ALL of: activation < COMEMORY_PRUNE_MIN_ACTIVATION (-2.0), Beta
  #     feedback <= COMEMORY_PRUNE_MIN_FEEDBACK (0.25), quality <=
  #     COMEMORY_PRUNE_BELOW_QUALITY (2), and zero incoming edges — OR
  #     superseded by a live memory with no access since the supersede edge.
  #   ghost_ref_memories: owners of a pinned --ref-symbol whose target is gone
  #     from a CURRENT index (advisory — never deleted by --apply).
  comemory prune --json";

/// Arguments to `comemory prune`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Execute the cleanup (soft-delete low-value memories, drop orphan
    /// edges + stale code symbols). Without this flag prune only scans and
    /// reports.
    #[arg(long, default_value_t = false)]
    pub apply: bool,
    /// Restrict --apply to these memory ids (comma-separated 8-hex ids).
    /// Ids that are not prune candidates are ignored. Without this flag
    /// --apply acts on every low-value candidate.
    #[arg(long, value_delimiter = ',')]
    pub ids: Vec<String>,
    /// `--limit` / `--offset` window over the dry-run `stale_code_files`
    /// and `low_value_memories` lists. The same window applies to BOTH.
    /// It windows DISPLAY ONLY: `--apply` always acts on the full
    /// candidate set regardless of `--limit` / `--offset`.
    #[command(flatten)]
    pub page: PaginationArgs,
}

/// Run `comemory prune`: build the request, delegate the scan (+ optional
/// apply) to `api::prune::run`, then emit. Always emits the report,
/// regardless of `--apply`.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let cfg = load_config(&paths)?;
    let mut conn = connection::open(paths.db_path())?;

    let req = api::prune::Request {
        apply: a.apply,
        limit: a.page.limit,
        offset: a.page.offset,
        ids: a.ids,
    };
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let report = api::prune::run(&mut ctx, req)?;
    output::emit(&report, json_flag)
}
