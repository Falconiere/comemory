//! Output helpers for `comemory prune`. JSON mode serialises the
//! [`Report`] struct directly (each list as a `Page` envelope); TTY mode
//! prints the orphan-edge count then each list's windowed entries followed
//! by a shared pagination footer carrying the list's full total. `Report`
//! also doubles as `maintenance::prune::run`'s return type — the CLI's `--json`
//! stdout and the `/api/v1/prune` response `data` field build from the
//! same owned value.

use std::io::Write as _;

use crate::cli::output::{json, tty};
use crate::domains::maintenance::retention_report::{PruneRow, Report};
use crate::prelude::*;
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
    write_page(
        &mut out,
        "stale_code_files",
        &report.stale_code_files,
        Clone::clone,
    )?;
    let render_row = |row: &PruneRow| {
        format!(
            "{} [{}] activation={:.2} age_days={} {}",
            row.id, row.reason, row.activation, row.age_days, row.title
        )
    };
    write_page(
        &mut out,
        "low_value_memories",
        &report.low_value_memories,
        render_row,
    )?;
    write_page(
        &mut out,
        "ghost_ref_memories",
        &report.ghost_ref_memories,
        render_row,
    )?;
    Ok(())
}

/// Write one labelled list section: a `label : <total>` header, each item
/// rendered by `render` and indented below it, then the shared
/// [`tty::write_page_footer`] showing the list's full total. One rendering
/// rule for every prune list shape (Binding Rule 1): string lists pass
/// [`Clone::clone`] as `render`, [`PruneRow`] lists pass the row format.
fn write_page<T>(
    out: &mut impl std::io::Write,
    label: &str,
    page: &Page<T>,
    render: impl Fn(&T) -> String,
) -> Result<()> {
    let total = page.total.unwrap_or(page.items.len());
    writeln!(out, "{label:<18} : {total}")?;
    for item in &page.items {
        writeln!(out, "  - {}", render(item))?;
    }
    tty::write_page_footer(out, page.items.len(), page.offset, page.total)
}

#[cfg(test)]
#[path = "tests/prune.rs"]
mod tests;
