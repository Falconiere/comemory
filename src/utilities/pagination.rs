//! Transport-neutral pagination: the window a caller asks for, the maths that
//! slices a result list to it, and the two envelopes that report the slice.
//!
//! Every paginated surface shares these four types so window semantics cannot
//! drift between `cli` and `serve` (Binding Rule 1):
//!
//! - [`PageWindow`] — the requested `(offset, limit)`, built from a
//!   subcommand's `--k`/`--limit` and `--offset` by [`page_window`].
//! - [`Page`] — the generic `{items, limit, offset, total, has_more}` JSON
//!   envelope every paged command serializes, sliced by [`Page::from_slice`].
//! - [`PageMeta`] — the cursor the retrieval envelopes (`search`, `find`,
//!   `search-code`, `context`) carry alongside their hits, built from a
//!   finished pipeline run by [`page_meta`].
//!
//! `limit == 0` is the "all" sentinel throughout: no slicing past `offset`,
//! and `has_more` is always `false`.

use serde::Serialize;

use crate::config::Config;

/// A paginated slice of `items` plus the cursor metadata describing the window
/// it was taken from. JSON-serializes to
/// `{ "items": [...], "limit": N, "offset": M, "total": T_or_null, "has_more": bool }`.
///
/// The type stores an already-sliced `items` vector; it never re-slices on
/// serialize. Build it with [`Page::from_slice`] (slice a full vector by a
/// window) or [`Page::new`] (you already sliced and know the metadata).
#[derive(Debug, Serialize)]
pub struct Page<T> {
    /// The page's items, already sliced to the window.
    pub items: Vec<T>,
    /// Requested window size. `0` is the sentinel for "all" (no slicing).
    pub limit: usize,
    /// Number of items skipped before this window started.
    pub offset: usize,
    /// Total number of items across all pages when known; `None` when the
    /// caller could not (or chose not to) count the full set.
    pub total: Option<usize>,
    /// Whether items exist beyond this window (`offset + items.len() < total`).
    pub has_more: bool,
}

impl<T> Page<T> {
    /// Build a `Page` from an already-sliced `items` vector plus the window
    /// metadata. Use this when slicing happens elsewhere (e.g. in SQL with a
    /// `LIMIT`/`OFFSET`); otherwise prefer [`Page::from_slice`], which derives
    /// `total` and `has_more` for you.
    pub fn new(
        items: Vec<T>,
        limit: usize,
        offset: usize,
        total: Option<usize>,
        has_more: bool,
    ) -> Self {
        Self {
            items,
            limit,
            offset,
            total,
            has_more,
        }
    }

    /// Slice `all` to the `(limit, offset)` window and record the full length
    /// as `total`. This is the canonical paginator so every command windows
    /// identically:
    ///
    /// - `offset` past the end yields an empty page (never panics).
    /// - `limit == 0` means "all": no slicing past `offset`, `has_more` is
    ///   always `false`, and `total` is the full length.
    /// - otherwise the window is `all[offset .. offset + limit]` (clamped to
    ///   the end) and `has_more` is `offset + items.len() < total`.
    pub fn from_slice(all: Vec<T>, limit: usize, offset: usize) -> Self {
        let total = all.len();
        let start = offset.min(total);
        let mut items: Vec<T> = all.into_iter().skip(start).collect();
        if limit != 0 && items.len() > limit {
            items.truncate(limit);
        }
        let has_more = start + items.len() < total;
        Self {
            items,
            limit,
            offset,
            total: Some(total),
            has_more,
        }
    }
}

/// The `(offset, limit)` slice a paginated retrieval should return from the
/// bounded ranked window. `limit == 0` is the "page size = remaining within
/// the window" sentinel (the same "all" rule [`Page`] uses, bounded in
/// `retrieval::pipeline` by `max_page_window`).
#[derive(Debug, Clone, Copy)]
pub struct PageWindow {
    /// Leading ranked results to skip before the page starts.
    pub offset: usize,
    /// Page size; `0` means "everything remaining within the window".
    pub limit: usize,
}

impl PageWindow {
    /// The full first page sized to `top_k` — the unpaginated default that
    /// reproduces the pre-pagination behavior (`offset = 0`, `limit =
    /// top_k`).
    pub fn top_k(cfg: &Config) -> Self {
        Self {
            offset: 0,
            limit: cfg.retrieval.top_k,
        }
    }

    /// Whether this window starts at the head of the ranked list: nothing is
    /// skipped before its first item, so the window covers an unbroken prefix
    /// of the ranking rather than a band out of the middle of it.
    ///
    /// Independent of `limit` — a zero-length or "all remaining" window still
    /// starts at the head when `offset` is `0`.
    pub fn is_head(self) -> bool {
        self.offset == 0
    }
}

/// Pagination cursor metadata carried alongside the hits in a retrieval
/// envelope. `total` is the in-window ranked count (the diversified list the
/// page was sliced from, capped by `max_page_window`), not a global match
/// count.
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

/// Build the retrieval [`PageWindow`] for a paginated subcommand from its
/// `--k`/`--limit` page size (`None` → configured `retrieval.top_k`) and
/// `--offset`. Shared by `search`, `search-code`, `context`, `find`, `edges`
/// and `consolidate` so no two commands drift on what "page size" means
/// (Binding Rule 1). `--k 0` / `--limit 0` is preserved as the "all remaining
/// within the window" sentinel.
pub(crate) fn page_window(cfg: &Config, k: Option<usize>, offset: usize) -> PageWindow {
    PageWindow {
        offset,
        limit: k.unwrap_or(cfg.retrieval.top_k),
    }
}

/// Translate a finished pipeline run's window metadata into the [`PageMeta`]
/// the JSON envelopes carry. Shared by the paginated retrieval commands so the
/// cursor shape stays uniform.
pub(crate) fn page_meta(window: PageWindow, has_more: bool, total: usize) -> PageMeta {
    PageMeta {
        limit: window.limit,
        offset: window.offset,
        has_more,
        total: Some(total),
    }
}

#[cfg(test)]
#[path = "tests/pagination.rs"]
mod tests;
