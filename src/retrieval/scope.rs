//! Created-date window applied to a retrieval run.
//!
//! Every leg filters candidates on `memories.created_at` against the same
//! [`TimeScope`] — "what did we know then" instead of "what do we know
//! now". Bounds are pre-normalized ISO-8601 strings from the CLI's
//! `--since` / `--until` / `--as-of` parsing, compared through SQLite
//! `datetime()` so mixed stored precision cannot invert the order.
//! [`Filters`] bundles the scope with the `repo` / `kind` filters.

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

/// Everything one retrieval run narrows candidates by: `repo` / `kind` +
/// the [`TimeScope`] + the domain scope, one value per leg instead of
/// four parameters. `Copy` is load-bearing — call sites pass by value,
/// then keep reading fields; removing the derive is a breaking refactor,
/// not a cleanup.
#[derive(Debug, Clone, Copy)]
pub struct Filters<'a> {
    /// Repo filter, or `None` to search every repo.
    pub repo: Option<&'a str>,
    /// Memory-kind filter (canonical lowercase, e.g. `decision`), or
    /// `None` to search every kind.
    pub kind: Option<&'a str>,
    /// Created-date window plus its `--as-of` semantics.
    pub scope: &'a TimeScope,
    /// Which of memory/document/code this run is scoped to. The field
    /// exists and every caller states it explicitly; nothing dispatches
    /// on it yet (each leg still runs unconditionally except
    /// [`crate::retrieval::doc_route`], which already checks it) — the
    /// pipeline wiring lands in a later step.
    pub domains: Domains,
}

impl<'a> Filters<'a> {
    /// Filter nothing: no repo, no kind, unbounded scope, every domain.
    /// Callers that narrow only some dimensions build on it with struct
    /// update syntax (`Filters { repo, ..Filters::none() }`).
    pub fn none() -> Self {
        Filters {
            repo: None,
            kind: None,
            scope: &UNBOUNDED,
            domains: Domains::all(),
        }
    }

    /// The scope's store-layer window — shorthand for `self.scope.window()`
    /// at the SQL call sites.
    pub fn window(&self) -> CreatedWindow<'a> {
        self.scope.window()
    }
}

/// One of the three domains a retrieval run can be scoped to. See
/// [`Domains`] for the bitmask that records "which of these are in
/// scope" on [`Filters`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    /// Hand-authored markdown memories (`memories` / `memory_fts` /
    /// `memory_vec`).
    Memory,
    /// Indexed external documents (`documents` / `document_chunks` /
    /// `document_fts`).
    Document,
    /// Extracted code symbols (`code_symbols` / `code_fts` / `code_vec`).
    Code,
}

impl Domain {
    /// This domain's bit within a [`Domains`] mask.
    const fn bit(self) -> u8 {
        match self {
            Domain::Memory => 0b001,
            Domain::Document => 0b010,
            Domain::Code => 0b100,
        }
    }
}

/// A `Copy` `u8`-bitmask over the three [`Domain`]s, not a `HashSet` —
/// [`Filters`] is deliberately `Copy` (see its doc comment), so every
/// field it carries must be too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Domains(u8);

impl Domains {
    /// Every domain in scope — the `search`/`context` CLI default.
    pub fn all() -> Self {
        Domains::of(&[Domain::Memory, Domain::Document, Domain::Code])
    }

    /// Memory only — pinned by `eval::runner` (behind `eval`, `tune`, and
    /// `bandit`, whose golden pairs are memory ids) and by
    /// `cli::search_only` for a memory-only `--only`.
    pub fn memory_only() -> Self {
        Domains::of(&[Domain::Memory])
    }

    /// Build a mask from an explicit list of domains, e.g. for the CLI
    /// `--only` flag. Duplicates are harmless (`|` is idempotent).
    pub fn of(domains: &[Domain]) -> Self {
        Domains(domains.iter().fold(0, |acc, d| acc | d.bit()))
    }

    /// Whether `domain` is set in this mask.
    pub fn contains(self, domain: Domain) -> bool {
        self.0 & domain.bit() != 0
    }
}

impl Default for Domains {
    /// Unscoped defaults to every domain, matching [`Filters::none`].
    fn default() -> Self {
        Domains::all()
    }
}

#[cfg(test)]
#[path = "tests/scope.rs"]
mod tests;
