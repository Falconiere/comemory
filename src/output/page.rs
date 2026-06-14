//! Shared pagination envelope for CLI commands. `Page<T>` is the generic
//! sibling of the command-specific `search`/`search-code` `Envelope`s: where
//! those carry retrieval-only fields (`hits`, `query_id`), `Page<T>` carries
//! the slice + the `limit`/`offset`/`total`/`has_more` cursor metadata that
//! every paginated command shares. Callers slice their full result list with
//! [`Page::from_slice`] so window math (and the `--limit 0` "all" rule) lives
//! in exactly one place (Binding Rule 1).
//!
//! JSON shape:
//! `{ "items": [...], "limit": N, "offset": M, "total": T_or_null, "has_more": bool }`.

use serde::Serialize;

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
