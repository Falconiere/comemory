//! Emitters for `comemory search-code`. The `--json` envelope and its `Row`
//! are the contract both transports serialize, so they live in
//! [`crate::domains::retrieval::code_search_result`]; this module only writes
//! them out. TTY mode emits one
//! `score path:start-end symbol (kind) #id` line per hit — the trailing
//! `#<symbol_id>` is the id `comemory feedback --used-code` takes — plus
//! the shared query footer in its code flavor.

use std::io::Write;

use crate::cli::output::{json, tty};
use crate::domains::retrieval::code_rerank::CodeReranked;
use crate::domains::retrieval::code_search_result::envelope;
use crate::domains::retrieval::learned_report::LearnedOrdering;
use crate::prelude::*;
use crate::utilities::pagination::PageMeta;

/// Render `hits` to stdout in either JSON or TTY mode. `query_id` is the
/// retrieval_log id for this run (JSON field / TTY footer); `None` skips
/// it. `index_empty` is the CLI layer's "no `code_symbols` rows at all"
/// probe: in TTY mode a zero-hit result over an empty index prints a
/// `comemory index-code` hint instead of silent emptiness. JSON mode
/// ignores it — machine consumers read `hits: []` directly.
pub fn emit(
    hits: &[CodeReranked],
    query_id: Option<&str>,
    page: PageMeta,
    index_empty: bool,
    json_flag: bool,
    learned: Option<&LearnedOrdering>,
) -> Result<()> {
    if json_flag {
        return json::write(&envelope(hits, query_id, page, learned));
    }
    let mut out = std::io::stdout().lock();
    write_tty(&mut out, hits, query_id, index_empty)?;
    tty::write_learned_line(&mut out, learned)
}

/// Render the TTY view of `hits` to `out`. Public so tests can capture
/// the output without going through stdout. The `query: <qid>` footer
/// semantics live in [`tty::write_query_footer`] (code flavor, so the
/// feedback hint references `--used-code`).
pub fn write_tty(
    out: &mut impl Write,
    hits: &[CodeReranked],
    query_id: Option<&str>,
    index_empty: bool,
) -> Result<()> {
    for hit in hits {
        writeln!(
            out,
            "{}  {}:{}-{}  {}  ({})  {}",
            tty::score(hit.parts.final_score),
            hit.path,
            hit.line_start,
            hit.line_end,
            hit.symbol,
            hit.kind,
            tty::dim(&format!("#{}", hit.symbol_id)),
        )?;
    }
    if hits.is_empty() && index_empty {
        writeln!(
            out,
            "no code indexed yet — run `comemory index-code --repo <name> --path <repo>` first"
        )?;
    }
    tty::write_query_footer(out, query_id, !hits.is_empty(), tty::FeedbackHint::Code)
}
