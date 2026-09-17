//! Output helpers for `comemory prune`. JSON mode serialises the
//! [`Report`] struct directly (each list as a `Page` envelope); TTY mode
//! prints the orphan-edge count then each list's windowed entries followed
//! by a shared pagination footer carrying the list's full total. `Report`
//! also doubles as `api::prune::run`'s return type — the CLI's `--json`
//! stdout and the `/api/v1/prune` response `data` field build from the
//! same owned value.

use std::io::Write as _;

use crate::output::{json, tty};
use crate::prelude::*;
use crate::prune::report::{PruneRow, Report};
use crate::utilities::pagination::Page;

/// Render `report` to stdout in either JSON or TTY mode.
pub fn emit(report: &Report, json_flag: bool) -> Result<()> {
    if json_flag {
        json::write(report)?;
        return Ok(());
    }
    let mut out = std::io::stdout().lock();
    writeln!(out, "orphan_edges       : {}", report.orphan_edges)?;
    writeln!(out, "trash_count        : {}", report.trash_count)?;
    writeln!(out, "reclaimable_bytes  : {}", report.reclaimable_bytes)?;
    write_list(&mut out, "stale_code_files", &report.stale_code_files)?;
    write_rows(&mut out, "low_value_memories", &report.low_value_memories)?;
    write_rows(&mut out, "ghost_ref_memories", &report.ghost_ref_memories)?;
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

/// Write one labelled [`PruneRow`] list section: a `label : <total>` header,
/// each row's id/reason/activation/age_days/title indented below it, then
/// the shared [`tty::write_page_footer`]. Mirrors [`write_list`] for the two
/// row-shaped lists (Binding Rule 1: one rendering rule per list shape).
fn write_rows(out: &mut impl std::io::Write, label: &str, page: &Page<PruneRow>) -> Result<()> {
    let total = page.total.unwrap_or(page.items.len());
    writeln!(out, "{label:<18} : {total}")?;
    for row in &page.items {
        writeln!(
            out,
            "  - {} [{}] activation={:.2} age_days={} {}",
            row.id, row.reason, row.activation, row.age_days, row.title
        )?;
    }
    tty::write_page_footer(out, page.items.len(), page.offset, page.total)
}

#[cfg(test)]
#[path = "tests/prune.rs"]
mod tests;
