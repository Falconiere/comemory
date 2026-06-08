//! Output helpers for `comemory prune`. JSON mode serialises the
//! [`crate::cli::prune::Report`] struct directly; TTY mode renders a
//! two-line summary of orphan-edge count and stale code file count.

use std::io::Write as _;

use crate::cli::prune::Report;
use crate::output::json;
use crate::prelude::*;

/// Render `report` to stdout in either JSON or TTY mode.
pub fn emit(report: &Report, json_flag: bool) -> Result<()> {
    if json_flag {
        json::write(report)?;
        return Ok(());
    }
    let mut out = std::io::stdout().lock();
    writeln!(out, "orphan_edges     : {}", report.orphan_edges)?;
    writeln!(out, "stale_code_files : {}", report.stale_code_files.len())?;
    for path in &report.stale_code_files {
        writeln!(out, "  - {path}")?;
    }
    Ok(())
}
