//! Output helpers for `comemory prune`. JSON mode serialises the
//! [`crate::cli::prune::Report`] struct directly (each list as a `Page`
//! envelope); TTY mode prints the orphan-edge count then each list's
//! windowed entries followed by a shared pagination footer carrying the
//! list's full total.

use std::io::Write as _;

use crate::cli::prune::Report;
use crate::output::page::Page;
use crate::output::{json, tty};
use crate::prelude::*;

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
