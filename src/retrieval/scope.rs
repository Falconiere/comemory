//! Created-date window applied to a retrieval run.
//!
//! Every leg of the pipeline (lexical, vector, graph) filters candidates
//! on `memories.created_at` against the same [`TimeScope`], so a scoped
//! run answers "what did we know then" instead of "what do we know now".
//! Bounds are pre-normalized ISO-8601 strings produced by the CLI's
//! `--since` / `--until` / `--as-of` parsing; the store layer compares
//! them through SQLite `datetime()` so mixed stored precision cannot
//! invert the order.

/// A created-date window over the memory corpus.
///
/// [`TimeScope::none`] is the unbounded default and reproduces
/// pre-time-travel behavior bit-for-bit: both bounds bind NULL, which
/// every predicate short-circuits on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TimeScope {
    /// Inclusive lower bound (`--since`): keep memories created at or
    /// after this instant. `None` leaves the window open on the left.
    pub since: Option<String>,
    /// Inclusive upper bound (`--until` or `--as-of`): keep memories
    /// created at or before this instant. `None` leaves the window open
    /// on the right.
    pub cutoff: Option<String>,
    /// Whether `cutoff` came from `--as-of` rather than `--until`. Only
    /// `--as-of` additionally scopes the supersede penalty, so that a
    /// memory is penalized solely by superseders that already existed at
    /// the cutoff; `--until` filters candidates and nothing else.
    pub as_of: bool,
}

impl TimeScope {
    /// The unbounded scope: no filtering, no supersede scoping.
    pub fn none() -> Self {
        Self::default()
    }

    /// Whether this scope constrains nothing, so callers can skip the
    /// scoped code paths entirely. `as_of` is not consulted: it only
    /// refines what `cutoff` means, and a scope built from `--as-of`
    /// always carries a cutoff.
    pub fn is_unbounded(&self) -> bool {
        self.since.is_none() && self.cutoff.is_none()
    }
}
