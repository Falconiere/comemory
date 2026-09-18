//! The owned value `comemory search-code` produces, and the `hits` envelope
//! both delivery adapters serialize it into.
//!
//! `retrieval::search_code::run` returns [`SearchCodeResult`][sc] so the CLI
//! writers and the `/api/v1/code/search` handler each build their own view
//! from one value — and both build it with the same [`envelope`][en].
//! Retrieval's result model, not a rendering concern (#166, #171);
//! `output::search_code` keeps the emitters.
//!
//! [en]: crate::domains::retrieval::code_search_result::envelope
//! [sc]: crate::domains::retrieval::code_search_result::SearchCodeResult
//!
//! JSON shape is `{"hits":[{"symbol_id":..,"repo":..,"path":..,"symbol":..,
//! "kind":..,"lang":..,"lines":[start,end],"score":..,"source":..,
//! "score_parts":{..}}],"query_id"?:..}`. `lines` serializes the
//! `(line_start, line_end)` pair as a 2-element `[start, end]` array — a
//! stable contract, pinned in `tests/output__search_code.rs`. `score_parts`
//! is the code-side explainability surface ([`CodeScoreParts`]), not debug
//! info.

use serde::Serialize;

use crate::domains::retrieval::code_rerank::{CodeReranked, CodeScoreParts};
use crate::domains::retrieval::learned_report::LearnedOrdering;
use crate::domains::retrieval::search_result::source_label;
use crate::utilities::pagination::PageMeta;

/// Everything `comemory search-code`'s render layer needs: `retrieval::search_code::run`
/// returns this so the CLI and the HTTP handler can each build their own
/// envelope (`--json` stdout vs the `/api/v1` response `data` field) from
/// one owned value instead of four loose parameters.
pub struct SearchCodeResult {
    /// Reranked hits for the requested page.
    pub hits: Vec<CodeReranked>,
    /// Id of the `retrieval_log` row for this run; `None` when tracking was
    /// off or logging failed.
    pub query_id: Option<String>,
    /// Pagination cursor for the returned page.
    pub meta: PageMeta,
    /// `true` when the page is empty because `code_symbols` has no rows at
    /// all (as opposed to a query that simply matched nothing) — the CLI
    /// TTY view uses this to print an `index-code` hint instead of silent
    /// emptiness.
    pub index_empty: bool,
    /// What the optional learned ordering stage did, when one ran.
    pub learned: Option<LearnedOrdering>,
}

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
    /// Reranked hits in final pipeline order for the requested page.
    pub hits: Vec<Row<'a>>,
    /// Id of the retrieval_log row for this run; absent when logging
    /// was off or failed. Feed it back via
    /// `comemory feedback <id> --used-code <ids>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_id: Option<&'a str>,
    /// Requested page size.
    pub limit: usize,
    /// Number of leading ranked results skipped.
    pub offset: usize,
    /// Whether more in-window ranked results exist beyond this page.
    pub has_more: bool,
    /// In-window ranked count (post-coalesce) the page was sliced from;
    /// `None` when not cheaply known. Not a global match count.
    pub total: Option<usize>,
    /// What the optional learned ordering stage did. Absent entirely when no
    /// stage ran, so a default build emits exactly the JSON it always has.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub learned: Option<&'a LearnedOrdering>,
}

/// Build the serializable envelope. Public so both delivery adapters and the
/// mirror tests can pin the JSON contract without going through stdout.
pub fn envelope<'a>(
    hits: &'a [CodeReranked],
    query_id: Option<&'a str>,
    page: PageMeta,
    learned: Option<&'a LearnedOrdering>,
) -> Envelope<'a> {
    Envelope {
        hits: hits.iter().map(row_from).collect(),
        query_id,
        limit: page.limit,
        offset: page.offset,
        has_more: page.has_more,
        total: page.total,
        learned,
    }
}

/// Project one reranked code hit into its serializable row.
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
