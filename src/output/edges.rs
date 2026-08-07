//! Output helpers for `comemory edges` — the lexical view of the relation
//! graph.
//!
//! JSON mode emits the shared [`Page`] envelope over one row per matched
//! triplet (the same `limit` / `offset` / `total` / `has_more` cursor every
//! paginated command carries); TTY mode renders each relation as
//! `src —rel→ dst  (weight n)` followed by the standard pagination footer.
//! Each row carries both the raw `edges` columns — so another subcommand can
//! address the endpoints by id — and the rendered text that actually matched.

use std::io::Write;

use serde::Serialize;

use crate::output::page::Page;
use crate::output::{json, tty};
use crate::prelude::*;
use crate::store::edge_fts::EdgeFtsHit;

/// Everything `comemory edges`'s render layer needs: `api::edges::run`
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

/// Build the serializable envelope. Public so tests can pin the JSON
/// contract without going through stdout.
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
/// because [`envelope`] never counts the full match set.
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
