//! The owned value `comemory search` produces, and the `hits` envelope both
//! delivery adapters serialize it into.
//!
//! `retrieval::search::run` returns [`SearchResult`][sr] so the CLI's
//! `--json`/TTY writers and the `/api/v1/search` handler each build their own
//! view from one value instead of five loose parameters — and both build it
//! with the same [`envelope`][en]. That shape is a contract shared by the two
//! transports, not a rendering concern, so it belongs to retrieval rather
//! than to `output` (#166, #171); `output::search` keeps the emitters, the
//! writers that actually touch stdout.
//!
//! [en]: crate::domains::retrieval::search_result::envelope
//! [sr]: crate::domains::retrieval::search_result::SearchResult
//!
//! Each hit carries `memory_id`, `score`, `source`
//! (`vector`|`lexical`|`hybrid`|`graph`), `tier` (1..4), optional
//! `superseded_by`, the `score_parts` object, and the navigation fields
//! `path` / `title` / `repo` / `kind` / `tags` / `references`. `score_parts`
//! is a stable explainability contract (M2 tuning reads it), not debug info;
//! the navigation fields are additive.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

use crate::domains::memories::References;
use crate::domains::memories::nav::{abs_path, title_of};
use crate::domains::retrieval::rerank::{Reranked, ScoreParts};
use crate::domains::retrieval::router::Source;
use crate::domains::retrieval::scope::{ScopeEcho, TimeScope};
use crate::store::memory_meta::MemoryMeta;
use crate::utilities::pagination::PageMeta;

/// Everything `comemory search`'s render layer needs: `retrieval::search::run`
/// returns this so the CLI and the HTTP handler can each build their own
/// envelope (`--json` stdout vs the `/api/v1` response `data` field) from
/// one owned value instead of five loose parameters.
pub struct SearchResult {
    /// Reranked + diversified hits for the requested page.
    pub hits: Vec<Reranked>,
    /// Id of the retrieval_log row for this run.
    pub query_id: Option<String>,
    /// Pagination cursor for the returned page.
    pub meta: PageMeta,
    /// Batched navigation metadata for `hits`, keyed by memory id.
    pub nav: HashMap<String, MemoryMeta>,
    /// The run's time-scoping flags.
    pub scope: TimeScope,
}

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
    /// The run's time-scoping flags, echoed at the envelope root. Every
    /// field is skipped when unset, so an unscoped run is unchanged.
    #[serde(flatten)]
    pub scope: ScopeEcho<'a>,
}

/// Build the serializable envelope. Public so both delivery adapters and
/// the snapshot tests can pin the JSON contract without going through
/// stdout. `meta` carries the batched navigation metadata (keyed by memory
/// id) and `data_dir` resolves each row's stored `md_path` into an absolute
/// path. `scope` echoes the time-scoping flags for this run
/// ([`ScopeEcho::default`] for an unscoped one).
pub fn envelope<'a, S: ::std::hash::BuildHasher>(
    hits: &'a [Reranked],
    query_id: Option<&'a str>,
    page: PageMeta,
    meta: &HashMap<String, MemoryMeta, S>,
    data_dir: &Path,
    scope: ScopeEcho<'a>,
) -> Envelope<'a> {
    Envelope {
        hits: hits.iter().map(|h| row_from(h, meta, data_dir)).collect(),
        query_id,
        limit: page.limit,
        offset: page.offset,
        has_more: page.has_more,
        total: page.total,
        scope,
    }
}

/// Build one [`Row`] for `h`, enriching it with navigation fields from
/// `meta` (keyed by memory id). A missing entry (raced soft-delete / rebuild)
/// degrades to empty path/kind/tags and an absent repo; `title` always comes
/// from the body, which the rerank stage carries inline.
fn row_from<'a, S: ::std::hash::BuildHasher>(
    h: &'a Reranked,
    meta: &HashMap<String, MemoryMeta, S>,
    data_dir: &Path,
) -> Row<'a> {
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

/// Stable lowercase label for a retrieval [`Source`], shared with
/// [`crate::domains::retrieval::code_search_result`] and both TTY writers so
/// every `source` field agrees on one vocabulary.
pub fn source_label(s: Source) -> &'static str {
    match s {
        Source::Vector => "vector",
        Source::Lexical => "lexical",
        Source::Hybrid => "hybrid",
        Source::Graph => "graph",
    }
}

#[cfg(test)]
#[path = "tests/search_result.rs"]
mod tests;
