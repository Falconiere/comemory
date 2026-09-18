//! Writers for `comemory edges` — the lexical view of the relation graph.
//!
//! JSON mode emits the shared [`Page`](crate::utilities::pagination::Page)
//! envelope built by
//! [`edges_result::envelope`](crate::domains::graph::edges_result::envelope) —
//! one row per matched triplet, carrying the same `limit` / `offset` / `total` /
//! `has_more` cursor every paginated command does. TTY mode renders each
//! relation as `src —rel→ dst  (weight n)` followed by the standard pagination
//! footer. The row shape itself belongs to the graph capability, because
//! `GET /api/v1/edges` serializes the same one; this module only writes it out.

use std::io::Write;

use crate::cli::output::{json, tty};
use crate::domains::graph::edges_result::envelope;
use crate::prelude::*;
use crate::store::edge_fts::EdgeFtsHit;

/// Render `hits` to stdout in either JSON or TTY mode.
pub fn emit(
    hits: &[EdgeFtsHit],
    limit: usize,
    offset: usize,
    has_more: bool,
    json_flag: bool,
) -> Result<()> {
    if json_flag {
        return json::write(&envelope(hits, limit, offset, has_more));
    }
    write_tty(&mut std::io::stdout().lock(), hits, offset)
}

/// Render the TTY view of `hits` to `out`. Public so tests can capture the
/// output without going through stdout. The footer reports `?` for the total
/// because [`crate::domains::graph::edges_result::envelope`] never counts the
/// full match set.
pub fn write_tty(out: &mut impl Write, hits: &[EdgeFtsHit], offset: usize) -> Result<()> {
    for h in hits {
        let weight = tty::dim(&format!("(weight {})", h.weight));
        writeln!(
            out,
            "{} \u{2014}{}\u{2192} {}  {weight}",
            h.src_text, h.rel, h.dst_text
        )?;
    }
    tty::write_page_footer(out, hits.len(), offset, None)
}

#[cfg(test)]
#[path = "tests/edges.rs"]
mod tests;
