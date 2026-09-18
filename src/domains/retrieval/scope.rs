//! Created-date window applied to a retrieval run.
//!
//! Every leg filters candidates on `memories.created_at` against the same
//! [`TimeScope`] — "what did we know then" instead of "what do we know
//! now". Bounds are pre-normalized ISO-8601 strings from the CLI's
//! `--since` / `--until` / `--as-of` parsing, compared through SQLite
//! `datetime()` so mixed stored precision cannot invert the order.
//! [`Filters`] bundles the scope with the `repo` / `kind` filters,
//! [`Domains`][dm] records which corpora a run may touch, and
//! [`ScopeEcho`][se] is the transport-neutral echo both delivery adapters put
//! at the root of a `--json` / `/api/v1` envelope.
//!
//! [dm]: crate::domains::retrieval::scope::Domains
//! [se]: crate::domains::retrieval::scope::ScopeEcho

use serde::Serialize;

use crate::prelude::*;
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
    /// supersede penalty in [`crate::domains::retrieval::rerank`], never candidate
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
    /// [`crate::domains::retrieval::doc_route`], which already checks it) — the
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

/// Resolve a `--only` selection plus `--kind` into the [`Domains`] mask a
/// search run should use. Transport-neutral: the caller maps its own flag
/// type onto [`Domain`] first, so the clap `ValueEnum` stays in `cli`.
///
/// `only` empty: `--kind` narrows the default to memory-only, else the full
/// default stands. `only` non-empty: taken verbatim, rejected as a usage
/// error if it excludes memory while `kind` is set (nothing to apply the
/// filter to), if it includes [`Domain::Code`] — no leg here searches code
/// yet, so it would otherwise silently drop code results instead of finding
/// any — or if it includes both [`Domain::Memory`] and [`Domain::Document`]:
/// `cli::search::run` routes on `Memory` alone (memory present ->
/// `run_memory`, which never reads `Filters.domains` and would silently drop
/// the document half of the request).
pub fn resolve_domains(only: &[Domain], kind: Option<&str>) -> Result<Domains> {
    if only.is_empty() {
        return Ok(if kind.is_some() {
            Domains::memory_only()
        } else {
            Domains::all()
        });
    }
    let domains = Domains::of(only);
    if let Some(k) = kind
        && !domains.contains(Domain::Memory)
    {
        return Err(Error::Usage(format!(
            "--kind {k} requires memory in --only (got: {})",
            only_label(only)
        )));
    }
    if domains.contains(Domain::Code) {
        return Err(Error::Usage(format!(
            "code domain joins unified search in a later release; use `comemory search-code` \
             instead (got: --only {})",
            only_label(only)
        )));
    }
    if domains.contains(Domain::Memory) && domains.contains(Domain::Document) {
        return Err(Error::Usage(format!(
            "memory and document can't be combined yet — search unifies them in a later \
             release; run them separately (got: --only {})",
            only_label(only)
        )));
    }
    Ok(domains)
}

/// Render `only` back as its comma-separated flag values, for the
/// contradiction errors in [`resolve_domains`]. The three names are the
/// `--only` flag's own lowercase spellings.
fn only_label(only: &[Domain]) -> String {
    only.iter()
        .map(|d| match d {
            Domain::Memory => "memory",
            Domain::Document => "document",
            Domain::Code => "code",
        })
        .collect::<Vec<_>>()
        .join(",")
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

#[cfg(test)]
#[path = "tests/scope.rs"]
mod tests;
