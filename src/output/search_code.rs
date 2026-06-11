//! Output helpers for `comemory search-code`. JSON shape is
//! `{"hits":[{"symbol_id":..,"repo":..,"path":..,"symbol":..,"kind":..,
//! "lang":..,"lines":[start,end],"score":..,"source":..,
//! "score_parts":{..}}],"query_id"?:..}`. `lines` serializes the
//! `(line_start, line_end)` pair as a 2-element `[start, end]` array —
//! a stable contract, pinned in `tests/output/search_code.rs`.
//! `score_parts` is the code-side explainability surface
//! ([`CodeScoreParts`]), not debug info. TTY mode emits one
//! `score path:start-end symbol (kind) #id` line per hit — the trailing
//! `#<symbol_id>` is the id `comemory feedback --used-code` takes — plus
//! the shared query footer in its code flavor.

use std::io::Write;

use serde::Serialize;

use crate::output::search::source_label;
use crate::output::{json, tty};
use crate::prelude::*;
use crate::retrieval::code_rerank::{CodeReranked, CodeScoreParts};

/// One code hit as emitted to the user. `score` duplicates
/// `score_parts.final_score` so simple consumers never need to descend
/// into the parts object.
#[derive(Serialize)]
pub struct Row<'a> {
    /// `code_symbols.id` of the hit (the parent's id for a coalesced
    /// cAST chunk win) — the id `comemory feedback --used-code` takes.
    pub symbol_id: i64,
    /// Repository the symbol was indexed from.
    pub repo: &'a str,
    /// Repo-relative file path.
    pub path: &'a str,
    /// Qualified symbol name.
    pub symbol: &'a str,
    /// Symbol kind, e.g. `function`.
    pub kind: &'a str,
    /// Source language, e.g. `rust`.
    pub lang: &'a str,
    /// `[line_start, line_end]` of the match (a tuple serializes as a
    /// JSON array).
    pub lines: (i64, i64),
    /// Final blended score (`score_parts.final_score`).
    pub score: f64,
    /// Which retrieval branch produced the hit.
    pub source: &'static str,
    /// Every multiplicative factor behind `score` (stable contract).
    pub score_parts: &'a CodeScoreParts,
}

/// JSON envelope returned to `--json` callers. Wraps the hits under `hits`
/// so future top-level fields can be added without breaking parsers,
/// mirroring the `comemory search` envelope.
#[derive(Serialize)]
pub struct Envelope<'a> {
    /// Reranked hits in final pipeline order.
    pub hits: Vec<Row<'a>>,
    /// Id of the retrieval_log row for this run; absent when logging
    /// was off or failed. Feed it back via
    /// `comemory feedback <id> --used-code <ids>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_id: Option<&'a str>,
}

/// Build the serializable envelope. Public so mirror tests can pin the
/// JSON contract without going through stdout.
pub fn envelope<'a>(hits: &'a [CodeReranked], query_id: Option<&'a str>) -> Envelope<'a> {
    Envelope {
        hits: hits.iter().map(row_from).collect(),
        query_id,
    }
}

/// Render `hits` to stdout in either JSON or TTY mode. `query_id` is the
/// retrieval_log id for this run (JSON field / TTY footer); `None` skips
/// it. `index_empty` is the CLI layer's "no `code_symbols` rows at all"
/// probe: in TTY mode a zero-hit result over an empty index prints a
/// `comemory index-code` hint instead of silent emptiness. JSON mode
/// ignores it — machine consumers read `hits: []` directly.
pub fn emit(
    hits: &[CodeReranked],
    query_id: Option<&str>,
    index_empty: bool,
    json_flag: bool,
) -> Result<()> {
    if json_flag {
        return json::write(&envelope(hits, query_id));
    }
    write_tty(&mut std::io::stdout().lock(), hits, query_id, index_empty)
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
            tty::score(hit.parts.final_score as f32),
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

fn row_from(h: &CodeReranked) -> Row<'_> {
    Row {
        symbol_id: h.symbol_id,
        repo: h.repo.as_str(),
        path: h.path.as_str(),
        symbol: h.symbol.as_str(),
        kind: h.kind.as_str(),
        lang: h.lang.as_str(),
        lines: (h.line_start, h.line_end),
        score: h.parts.final_score,
        source: source_label(h.source),
        score_parts: &h.parts,
    }
}
