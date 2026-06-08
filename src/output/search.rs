//! Output helpers for `comemory search`. JSON shape is
//! `{"hits":[{"memory_id":..,"score":..,"source":"vector"|"lexical"}]}`.
//! TTY mode emits one hit per line with a colored score prefix.

use std::io::Write as _;

use serde::Serialize;

use crate::output::{json, tty};
use crate::prelude::*;
use crate::retrieval::router::{RoutedHit, Source};

/// Per-hit row used for both the JSON envelope and the TTY renderer.
#[derive(Serialize)]
struct Row<'a> {
    memory_id: &'a str,
    score: f32,
    source: &'static str,
}

/// JSON envelope returned to `--json` callers. Wraps the hits under `hits`
/// so future top-level fields (route, filters, ...) can be added without
/// breaking parsers.
#[derive(Serialize)]
struct Envelope<'a> {
    hits: Vec<Row<'a>>,
}

/// Render `hits` to stdout in either JSON or TTY mode.
pub fn emit(hits: &[RoutedHit], json_flag: bool) -> Result<()> {
    let rows: Vec<Row<'_>> = hits.iter().map(row_from).collect();
    if json_flag {
        let env = Envelope { hits: rows };
        json::write(&env)?;
        return Ok(());
    }
    let mut out = std::io::stdout().lock();
    for row in &rows {
        writeln!(
            out,
            "{}  {}  {}",
            tty::score(row.score),
            row.source,
            row.memory_id
        )?;
    }
    Ok(())
}

fn row_from(h: &RoutedHit) -> Row<'_> {
    Row {
        memory_id: h.memory_id.as_str(),
        score: h.score,
        source: source_label(h.source),
    }
}

fn source_label(s: Source) -> &'static str {
    match s {
        Source::Vector => "vector",
        Source::Lexical => "lexical",
    }
}
