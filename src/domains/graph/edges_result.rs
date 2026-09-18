//! The owned value `comemory edges` produces, and the `--json` envelope both
//! transports serialize from it.
//!
//! [`edges::run`](crate::domains::graph::edges::run) returns
//! [`EdgesResult`](crate::domains::graph::edges_result::EdgesResult);
//! [`envelope`](crate::domains::graph::edges_result::envelope) turns it into the
//! shared [`Page`](crate::utilities::pagination::Page) shape that the CLI writes to
//! stdout and the `/api/v1/edges` handler nests inside its own envelope. Both
//! live here, beside the algorithm that produces them, because the row shape is
//! the graph capability's contract rather than a rendering concern — the same
//! split #171 made for `search`, `search_code` and `context`. The CLI's
//! `cli::output::edges` keeps only the writers.

use serde::Serialize;

use crate::store::edge_fts::EdgeFtsHit;
use crate::utilities::pagination::Page;

/// Everything `comemory edges`'s render layer needs: [`edges::run`](super::edges::run)
/// returns this so the CLI and the HTTP handler can each build their own
/// envelope from one owned value instead of four loose parameters.
pub struct EdgesResult {
    /// Matched triplet rows for the requested page.
    pub hits: Vec<EdgeFtsHit>,
    /// Requested page size.
    pub limit: usize,
    /// Number of leading ranked relations skipped.
    pub offset: usize,
    /// Whether more in-window ranked relations exist beyond this page.
    pub has_more: bool,
}

/// One matched relation as emitted under `--json`.
#[derive(Serialize)]
pub struct Row<'a> {
    /// Node kind of the edge source (`memory`, `file`, `symbol`, …).
    pub src_kind: &'a str,
    /// Source node id, exactly as stored in `edges`.
    pub src_id: &'a str,
    /// Rendered, indexed source text.
    pub src_text: &'a str,
    /// Relation kind (`supersedes`, `imports`, …).
    pub rel: &'a str,
    /// Node kind of the edge target.
    pub dst_kind: &'a str,
    /// Target node id, exactly as stored in `edges`.
    pub dst_id: &'a str,
    /// Rendered, indexed target text.
    pub dst_text: &'a str,
    /// Edge weight, as stored in `edges`.
    pub weight: i64,
    /// Negated BM25 — higher is better, matching the `search` convention.
    pub score: f32,
}

impl<'a> From<&'a EdgeFtsHit> for Row<'a> {
    fn from(h: &'a EdgeFtsHit) -> Self {
        Row {
            src_kind: &h.src_kind,
            src_id: &h.src_id,
            src_text: &h.src_text,
            rel: &h.rel,
            dst_kind: &h.dst_kind,
            dst_id: &h.dst_id,
            dst_text: &h.dst_text,
            weight: h.weight,
            score: h.score,
        }
    }
}

/// Build the serializable envelope shared by `comemory edges --json` and
/// `GET /api/v1/edges`.
///
/// `total` is deliberately `None`: the page is sliced in SQL behind a `k + 1`
/// probe, so `has_more` is known but the full match count never is — and
/// counting it would cost a second FTS scan for a number nothing uses.
pub fn envelope(hits: &[EdgeFtsHit], limit: usize, offset: usize, has_more: bool) -> Page<Row<'_>> {
    Page::new(
        hits.iter().map(Row::from).collect(),
        limit,
        offset,
        None,
        has_more,
    )
}

#[cfg(test)]
#[path = "tests/edges_result.rs"]
mod tests;
