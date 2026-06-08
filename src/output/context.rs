//! Output helpers for `comemory context`. JSON serializes the
//! [`crate::retrieval::bundle::Bundle`] verbatim; TTY mode prints a
//! human-readable summary of the matched memories and any code references
//! reached via the graph.

use std::io::Write as _;

use crate::output::{json, tty};
use crate::prelude::*;
use crate::retrieval::bundle::Bundle;

/// Render `bundle` to stdout in either JSON or TTY mode.
pub fn emit(bundle: &Bundle<'_>, json_flag: bool) -> Result<()> {
    if json_flag {
        return json::write(bundle);
    }
    tty::header(&format!("context: {}", bundle.query))?;
    let mut out = std::io::stdout().lock();
    for m in &bundle.memories {
        writeln!(
            out,
            "{}  {}  {}",
            tty::score(m.score),
            m.kind,
            tty::dim(&m.id)
        )?;
    }
    for c in &bundle.code_refs {
        writeln!(out, "  {}:{}:{}", c.repo, c.path, c.symbol)?;
    }
    Ok(())
}
