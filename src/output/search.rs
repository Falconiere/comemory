//! Output helpers for `comemory search`. JSON shape is
//! `{"hits":[{"memory_id":..,"score":..,"source":"vector"|"lexical"|"hybrid",
//! "superseded_by"?:..,"score_parts":{..}}]}`. `score_parts` is a stable
//! explainability contract (M2 tuning reads it), not debug info. TTY mode
//! emits one hit per line with a colored score prefix.

use std::io::Write as _;

use serde::Serialize;

use crate::output::{json, tty};
use crate::prelude::*;
use crate::retrieval::rerank::{Reranked, ScoreParts};
use crate::retrieval::router::Source;

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
    /// Live memory that supersedes this one, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<&'a str>,
    /// Every multiplicative factor behind `score` (stable contract).
    pub score_parts: &'a ScoreParts,
}

/// JSON envelope returned to `--json` callers. Wraps the hits under `hits`
/// so future top-level fields (route, filters, ...) can be added without
/// breaking parsers.
#[derive(Serialize)]
pub struct Envelope<'a> {
    /// Reranked hits in final pipeline order.
    pub hits: Vec<Row<'a>>,
    /// Id of the retrieval_log row for this run; absent when logging
    /// was off or failed. Feed it back via `comemory feedback <id>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_id: Option<&'a str>,
}

/// Build the serializable envelope. Public so snapshot tests can pin the
/// JSON contract without going through stdout.
pub fn envelope<'a>(hits: &'a [Reranked], query_id: Option<&'a str>) -> Envelope<'a> {
    Envelope {
        hits: hits.iter().map(row_from).collect(),
        query_id,
    }
}

/// Render `hits` to stdout in either JSON or TTY mode. `query_id` is the
/// retrieval_log id for this run (JSON field / TTY footer); `None` skips it.
pub fn emit(hits: &[Reranked], query_id: Option<&str>, json_flag: bool) -> Result<()> {
    if json_flag {
        return json::write(&envelope(hits, query_id));
    }
    let mut out = std::io::stdout().lock();
    for hit in hits {
        let suffix = match hit.superseded_by.as_deref() {
            Some(id) => format!(" (superseded by {id})"),
            None => String::new(),
        };
        writeln!(
            out,
            "{}  {}  {}{}",
            tty::score(hit.parts.final_score as f32),
            source_label(hit.source),
            hit.memory_id,
            suffix
        )?;
    }
    if let Some(qid) = query_id {
        writeln!(
            out,
            "query: {qid}  (feedback: comemory feedback {qid} --used <ids>)"
        )?;
    }
    Ok(())
}

fn row_from(h: &Reranked) -> Row<'_> {
    Row {
        memory_id: h.memory_id.as_str(),
        score: h.parts.final_score,
        source: source_label(h.source),
        superseded_by: h.superseded_by.as_deref(),
        score_parts: &h.parts,
    }
}

fn source_label(s: Source) -> &'static str {
    match s {
        Source::Vector => "vector",
        Source::Lexical => "lexical",
        Source::Hybrid => "hybrid",
    }
}
