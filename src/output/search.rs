//! Output helpers for `comemory search`. JSON shape is
//! `{"hits":[{"memory_id":..,"score":..,"source":"vector"|"lexical"|"hybrid",
//! "tier":1..4,"superseded_by"?:..,"score_parts":{..}}]}`. `score_parts` is a stable
//! explainability contract (M2 tuning reads it), not debug info. TTY mode
//! emits one hit per line with a colored score prefix.

use std::io::Write;

use serde::Serialize;

use crate::output::{json, tty};
use crate::prelude::*;
use crate::retrieval::rerank::{Reranked, ScoreParts};
use crate::retrieval::router::{Source, TIER_EXPANDED};

/// One search hit as emitted to the user. `score` duplicates
/// `score_parts.final_score` so simple consumers never need to descend
/// into the parts object.
#[derive(Serialize)]
pub struct Row<'a> {
    /// Identifier of the matched memory row.
    pub memory_id: &'a str,
    /// Final blended score (`score_parts.final_score`).
    pub score: f64,
    /// Which retrieval branch produced the hit.
    pub source: &'static str,
    /// Lexical ladder tier that produced the underlying candidate:
    /// 1 strict (also vector/hybrid default), 2 word-OR, 3 subtoken-OR,
    /// 4 learned expansion. Always serialized — a small int, no skip
    /// needed.
    pub tier: u8,
    /// Live memory that supersedes this one, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<&'a str>,
    /// Every multiplicative factor behind `score` (stable contract).
    pub score_parts: &'a ScoreParts,
}

/// Pagination cursor metadata carried alongside the hits in an
/// [`Envelope`]. `total` is the in-window ranked count (the diversified
/// list the page was sliced from, capped by `max_page_window`), not a
/// global match count.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct PageMeta {
    /// Requested page size (`--k` / `--limit`).
    pub limit: usize,
    /// Number of leading ranked results skipped (`--offset`).
    pub offset: usize,
    /// Whether more in-window ranked results exist beyond this page.
    pub has_more: bool,
    /// In-window ranked count the page was sliced from.
    pub total: Option<usize>,
}

/// JSON envelope returned to `--json` callers. Wraps the hits under `hits`
/// so future top-level fields (route, filters, ...) can be added without
/// breaking parsers. `hits` and `query_id` are unchanged from the
/// pre-pagination contract; `limit` / `offset` / `has_more` / `total` are
/// the pagination cursor (see [`PageMeta`]).
#[derive(Serialize)]
pub struct Envelope<'a> {
    /// Reranked hits in final pipeline order for the requested page.
    pub hits: Vec<Row<'a>>,
    /// Id of the retrieval_log row for this run; absent when logging
    /// was off or failed. Feed it back via `comemory feedback <id>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_id: Option<&'a str>,
    /// Requested page size.
    pub limit: usize,
    /// Number of leading ranked results skipped.
    pub offset: usize,
    /// Whether more in-window ranked results exist beyond this page.
    pub has_more: bool,
    /// In-window ranked count (diversified) the page was sliced from;
    /// `None` when not cheaply known.
    pub total: Option<usize>,
}

/// Build the serializable envelope. Public so snapshot tests can pin the
/// JSON contract without going through stdout.
pub fn envelope<'a>(
    hits: &'a [Reranked],
    query_id: Option<&'a str>,
    page: PageMeta,
) -> Envelope<'a> {
    Envelope {
        hits: hits.iter().map(row_from).collect(),
        query_id,
        limit: page.limit,
        offset: page.offset,
        has_more: page.has_more,
        total: page.total,
    }
}

/// Render `hits` to stdout in either JSON or TTY mode. `query_id` is the
/// retrieval_log id for this run (JSON field / TTY footer); `None` skips
/// it. `page` carries the pagination cursor for the JSON envelope.
pub fn emit(
    hits: &[Reranked],
    query_id: Option<&str>,
    page: PageMeta,
    json_flag: bool,
) -> Result<()> {
    if json_flag {
        return json::write(&envelope(hits, query_id, page));
    }
    write_tty(&mut std::io::stdout().lock(), hits, query_id)
}

/// Render the TTY view of `hits` to `out`. Public so tests can capture the
/// output without going through stdout. The `query: <qid>` footer semantics
/// live in [`tty::write_query_footer`], shared with `comemory context`.
pub fn write_tty(out: &mut impl Write, hits: &[Reranked], query_id: Option<&str>) -> Result<()> {
    for hit in hits {
        let suffix = match hit.superseded_by.as_deref() {
            Some(id) => format!(" (superseded by {id})"),
            None => String::new(),
        };
        // The expansion tier means the hit was only reachable via a mined
        // query expansion — flag it so users understand the looser match.
        let expanded = if hit.tier == TIER_EXPANDED {
            " [expanded]"
        } else {
            ""
        };
        writeln!(
            out,
            "{}  {}  {}{}{}",
            tty::score(hit.parts.final_score as f32),
            source_label(hit.source),
            hit.memory_id,
            suffix,
            expanded
        )?;
    }
    tty::write_query_footer(out, query_id, !hits.is_empty(), tty::FeedbackHint::Memory)
}

fn row_from(h: &Reranked) -> Row<'_> {
    Row {
        memory_id: h.memory_id.as_str(),
        score: h.parts.final_score,
        source: source_label(h.source),
        tier: h.tier,
        superseded_by: h.superseded_by.as_deref(),
        score_parts: &h.parts,
    }
}

/// Stable lowercase label for a retrieval [`Source`], shared with
/// `output::search_code` so the two `--json` envelopes agree on the
/// `source` vocabulary.
pub(crate) fn source_label(s: Source) -> &'static str {
    match s {
        Source::Vector => "vector",
        Source::Lexical => "lexical",
        Source::Hybrid => "hybrid",
    }
}
