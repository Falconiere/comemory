//! Output helpers for `comemory search`. Each hit carries
//! `memory_id`, `score`, `source` (`vector`|`lexical`|`hybrid`), `tier` (1..4),
//! optional `superseded_by`, the `score_parts` object, and the navigation
//! fields `path` / `title` / `repo` / `kind` / `tags` / `references`.
//! `score_parts` is a stable explainability contract (M2 tuning reads it), not
//! debug info; the navigation fields are additive. TTY mode emits one hit per
//! line with a colored score prefix plus a dim path/title line.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::memory::References;
use crate::output::{json, tty};
use crate::prelude::*;
use crate::retrieval::rerank::{Reranked, ScoreParts};
use crate::retrieval::router::{Source, TIER_EXPANDED};
use crate::store::memory_meta::MemoryMeta;

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
    /// Absolute path to the memory's markdown file (`data_dir` joined with
    /// the stored `md_path`). Empty when the row's metadata could not be
    /// resolved (raced soft-delete / rebuild).
    pub path: String,
    /// First non-empty trimmed line of the body — a human-readable title.
    /// Empty when the body is blank.
    pub title: String,
    /// Repo the memory belongs to, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Memory kind (decision|bug|convention|discovery|pattern|note); empty
    /// when the row's metadata could not be resolved.
    pub kind: String,
    /// Tag list from `memory_tags`.
    pub tags: Vec<String>,
    /// Code references harvested from the body (`{symbols, files}`).
    pub references: References,
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
/// JSON contract without going through stdout. `meta` carries the batched
/// navigation metadata (keyed by memory id) and `data_dir` resolves each
/// row's stored `md_path` into an absolute path.
pub fn envelope<'a>(
    hits: &'a [Reranked],
    query_id: Option<&'a str>,
    page: PageMeta,
    meta: &HashMap<String, MemoryMeta>,
    data_dir: &Path,
) -> Envelope<'a> {
    Envelope {
        hits: hits.iter().map(|h| row_from(h, meta, data_dir)).collect(),
        query_id,
        limit: page.limit,
        offset: page.offset,
        has_more: page.has_more,
        total: page.total,
    }
}

/// Render `hits` to stdout in either JSON or TTY mode. `query_id` is the
/// retrieval_log id for this run (JSON field / TTY footer); `None` skips
/// it. `page` carries the pagination cursor for the JSON envelope. `meta`
/// holds the per-hit navigation metadata and `data_dir` resolves each
/// markdown path to an absolute one.
pub fn emit(
    hits: &[Reranked],
    query_id: Option<&str>,
    page: PageMeta,
    json_flag: bool,
    meta: &HashMap<String, MemoryMeta>,
    data_dir: &Path,
) -> Result<()> {
    if json_flag {
        return json::write(&envelope(hits, query_id, page, meta, data_dir));
    }
    write_tty(
        &mut std::io::stdout().lock(),
        hits,
        query_id,
        meta,
        data_dir,
    )
}

/// Render the TTY view of `hits` to `out`. Public so tests can capture the
/// output without going through stdout. Each hit prints a score/source/id
/// line followed by a dim navigation line carrying the markdown path (and
/// title when present). The `query: <qid>` footer semantics live in
/// [`tty::write_query_footer`], shared with `comemory context`.
pub fn write_tty(
    out: &mut impl Write,
    hits: &[Reranked],
    query_id: Option<&str>,
    meta: &HashMap<String, MemoryMeta>,
    data_dir: &Path,
) -> Result<()> {
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
        let path = abs_path(meta.get(&hit.memory_id), data_dir);
        let title = title_of(&hit.body);
        let nav = if title.is_empty() {
            format!("    {path}")
        } else {
            format!("    {title} — {path}")
        };
        writeln!(out, "{}", tty::dim(&nav))?;
    }
    tty::write_query_footer(out, query_id, !hits.is_empty(), tty::FeedbackHint::Memory)
}

/// Build one [`Row`] for `h`, enriching it with navigation fields from
/// `meta` (keyed by memory id). A missing entry (raced soft-delete / rebuild)
/// degrades to empty path/kind/tags and an absent repo; `title` always comes
/// from the body, which the rerank stage carries inline.
fn row_from<'a>(h: &'a Reranked, meta: &HashMap<String, MemoryMeta>, data_dir: &Path) -> Row<'a> {
    let entry = meta.get(&h.memory_id);
    Row {
        memory_id: h.memory_id.as_str(),
        score: h.parts.final_score,
        source: source_label(h.source),
        tier: h.tier,
        superseded_by: h.superseded_by.as_deref(),
        score_parts: &h.parts,
        path: abs_path(entry, data_dir),
        title: title_of(&h.body),
        repo: entry.and_then(|m| m.repo.clone()),
        kind: entry.map(|m| m.kind.clone()).unwrap_or_default(),
        tags: entry.map(|m| m.tags.clone()).unwrap_or_default(),
        references: entry.map(|m| m.references.clone()).unwrap_or_default(),
    }
}

/// Resolve a memory's stored `md_path` against `data_dir` into an absolute
/// path string. Returns an empty string when the metadata is absent.
/// `Path::join` returns an absolute `md_path` unchanged and joins a relative
/// one, so this is correct whichever form the writer stored.
fn abs_path(entry: Option<&MemoryMeta>, data_dir: &Path) -> String {
    match entry {
        Some(m) => PathBuf::from(data_dir)
            .join(&m.md_path)
            .to_string_lossy()
            .into_owned(),
        None => String::new(),
    }
}

/// First non-empty trimmed line of `body` — a human-readable title. Empty
/// when the body has no non-blank line.
fn title_of(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_string()
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
