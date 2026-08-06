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
use crate::retrieval::scope::TimeScope;
use crate::store::memory_meta::MemoryMeta;

/// Everything `comemory search`'s render layer needs: `api::search::run`
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

/// The time-scoping flags echoed back at the root of a `--json` envelope,
/// flattened into both the `search` and `context` envelopes so the two
/// commands report a scoped run identically.
///
/// Every field is absent unless the corresponding flag was passed, which
/// keeps an unscoped envelope byte-identical to the pre-time-travel
/// contract. `until` and `as_of` are distinguished (they share
/// [`TimeScope::cutoff`]) so a consumer can tell whether the supersede
/// penalty was time-scoped too. Values are the normalized ISO-8601 bounds
/// the store actually compared against, not the raw flag text.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct ScopeEcho<'a> {
    /// Normalized `--since` bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<&'a str>,
    /// Normalized `--until` bound; `None` when the cutoff came from
    /// `--as-of` (or when there is no cutoff).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<&'a str>,
    /// Normalized `--as-of` bound; `None` when the cutoff came from
    /// `--until` (or when there is no cutoff).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of: Option<&'a str>,
}

impl<'a> ScopeEcho<'a> {
    /// The echo for `scope`: the cutoff is reported under `as_of` when the
    /// run carries as-of semantics and under `until` otherwise.
    pub fn of(scope: &'a TimeScope) -> Self {
        let cutoff = scope.cutoff.as_deref();
        ScopeEcho {
            since: scope.since.as_deref(),
            until: (!scope.as_of).then_some(cutoff).flatten(),
            as_of: scope.as_of.then_some(cutoff).flatten(),
        }
    }
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

/// Build the serializable envelope. Public so snapshot tests can pin the
/// JSON contract without going through stdout. `meta` carries the batched
/// navigation metadata (keyed by memory id) and `data_dir` resolves each
/// row's stored `md_path` into an absolute path. `scope` echoes the
/// time-scoping flags for this run ([`ScopeEcho::default`] for an
/// unscoped one).
pub fn envelope<'a>(
    hits: &'a [Reranked],
    query_id: Option<&'a str>,
    page: PageMeta,
    meta: &HashMap<String, MemoryMeta>,
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

/// Render `result` to stdout in either JSON or TTY mode. `data_dir` resolves
/// each markdown path to an absolute one. `result.scope` is echoed in the
/// JSON envelope only; the TTY view is unchanged by time scoping.
pub fn emit(result: &SearchResult, json_flag: bool, data_dir: &Path) -> Result<()> {
    let query_id = result.query_id.as_deref();
    if json_flag {
        return json::write(&envelope(
            &result.hits,
            query_id,
            result.meta,
            &result.nav,
            data_dir,
            ScopeEcho::of(&result.scope),
        ));
    }
    write_tty(
        &mut std::io::stdout().lock(),
        &result.hits,
        query_id,
        &result.nav,
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
/// one, so this is correct whichever form the writer stored. `pub(crate)`
/// so `serve::routes::memories`'s single-row lookup can resolve the same
/// path without duplicating the join logic.
pub(crate) fn abs_path(entry: Option<&MemoryMeta>, data_dir: &Path) -> String {
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
        Source::Graph => "graph",
    }
}
