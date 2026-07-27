//! Created-date window applied to a retrieval run.
//!
//! Every leg of the pipeline (lexical, vector, graph) filters candidates
//! on `memories.created_at` against the same [`TimeScope`], so a scoped
//! run answers "what did we know then" instead of "what do we know now".
//! Bounds are pre-normalized ISO-8601 strings produced by the CLI's
//! `--since` / `--until` / `--as-of` parsing; the store layer compares
//! them through SQLite `datetime()` so mixed stored precision cannot
//! invert the order.
//!
//! [`Filters`] bundles the scope with the `repo` / `kind` filters that
//! travel beside it, so one value carries everything a leg narrows
//! candidates by.

use crate::store::CreatedWindow;

/// The unbounded scope, borrowed by [`Filters::none`] so an unfiltered
/// caller needs no `TimeScope` binding of its own.
static UNBOUNDED: TimeScope = TimeScope {
    since: None,
    cutoff: None,
    as_of: false,
};

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

    /// Borrow the bounds as the store layer's [`CreatedWindow`] — the form
    /// every SQL predicate takes. `as_of` is dropped here: it governs the
    /// supersede penalty in [`crate::retrieval::rerank`], never candidate
    /// filtering.
    pub fn window(&self) -> CreatedWindow<'_> {
        CreatedWindow {
            since: self.since.as_deref(),
            cutoff: self.cutoff.as_deref(),
        }
    }

    /// The cutoff that scopes the supersede penalty: `Some` only under
    /// `--as-of`. Under a plain `--until` the penalty stays present-day —
    /// the flag filters candidates and nothing else.
    pub fn as_of_cutoff(&self) -> Option<&str> {
        if self.as_of {
            self.cutoff.as_deref()
        } else {
            None
        }
    }
}

/// Everything one retrieval run narrows candidates by: the `repo` / `kind`
/// filters and the [`TimeScope`], carried as a single value so each leg
/// takes one parameter instead of three.
#[derive(Debug, Clone, Copy)]
pub struct Filters<'a> {
    /// Repo filter, or `None` to search every repo.
    pub repo: Option<&'a str>,
    /// Memory-kind filter (canonical lowercase, e.g. `decision`), or
    /// `None` to search every kind.
    pub kind: Option<&'a str>,
    /// Created-date window plus its `--as-of` semantics.
    pub scope: &'a TimeScope,
}

impl<'a> Filters<'a> {
    /// Filter nothing: no repo, no kind, unbounded scope. Callers that
    /// narrow only some dimensions build on it with struct update syntax
    /// (`Filters { repo, ..Filters::none() }`).
    pub fn none() -> Self {
        Filters {
            repo: None,
            kind: None,
            scope: &UNBOUNDED,
        }
    }

    /// The scope's store-layer window — shorthand for `self.scope.window()`
    /// at the SQL call sites.
    pub fn window(&self) -> CreatedWindow<'a> {
        self.scope.window()
    }
}
