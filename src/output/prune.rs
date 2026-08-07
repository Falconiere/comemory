//! Output helpers for `comemory prune`. JSON mode serialises the
//! [`Report`] struct directly (each list as a `Page` envelope); TTY mode
//! prints the orphan-edge count then each list's windowed entries followed
//! by a shared pagination footer carrying the list's full total. `Report`
//! also doubles as `api::prune::run`'s return type — the CLI's `--json`
//! stdout and the `/api/v1/prune` response `data` field build from the
//! same owned value.

use std::io::Write as _;

use serde::Serialize;

use crate::output::page::Page;
use crate::output::{json, tty};
use crate::prelude::*;

/// Output schema for both JSON and TTY rendering.
#[derive(Serialize, Debug)]
pub struct Report {
    /// Count of `edges` rows whose source memory is missing or
    /// soft-deleted. A bare count (already a number, not a list), so it
    /// is never paginated.
    pub orphan_edges: i64,
    /// Paginated `<repo>:<path>` values whose corresponding `indexed_files`
    /// row has been removed. The repo prefix disambiguates identical paths
    /// across different repos (e.g. `src/main.rs` in two checkouts). The
    /// shared `--limit` / `--offset` window applies to the dry-run display
    /// only.
    pub stale_code_files: Page<String>,
    /// Paginated memory ids flagged by [`crate::prune::low_value::detect`]
    /// — soft-delete candidates (applied to the FULL set, not the page,
    /// when `apply` is set). The window applies to the dry-run display
    /// only.
    pub low_value_memories: Page<String>,
    /// Paginated memory ids flagged by [`crate::prune::stale_code::detect`]
    /// — owners of a pinned `references_symbol` whose target is a `ghost`
    /// (gone from a current index). Advisory: surfaced for the operator,
    /// never deleted by `apply` (spec Non-Goal 5). The window applies to
    /// display only.
    pub ghost_ref_memories: Page<String>,
}

/// Render `report` to stdout in either JSON or TTY mode.
pub fn emit(report: &Report, json_flag: bool) -> Result<()> {
    if json_flag {
        json::write(report)?;
        return Ok(());
    }
    let mut out = std::io::stdout().lock();
    writeln!(out, "orphan_edges       : {}", report.orphan_edges)?;
    write_list(&mut out, "stale_code_files", &report.stale_code_files)?;
    write_list(&mut out, "low_value_memories", &report.low_value_memories)?;
    write_list(&mut out, "ghost_ref_memories", &report.ghost_ref_memories)?;
    Ok(())
}

/// Write one labelled list section: a `label : <total>` header, the
/// page's windowed entries indented below it, then the shared
/// [`tty::write_page_footer`] showing the list's full total. Keeps the
/// two prune lists rendered identically (Binding Rule 1).
fn write_list(out: &mut impl std::io::Write, label: &str, page: &Page<String>) -> Result<()> {
    let total = page.total.unwrap_or(page.items.len());
    writeln!(out, "{label:<18} : {total}")?;
    for entry in &page.items {
        writeln!(out, "  - {entry}")?;
    }
    tty::write_page_footer(out, page.items.len(), page.offset, page.total)
}

#[cfg(test)]
#[path = "tests/prune.rs"]
mod tests;
