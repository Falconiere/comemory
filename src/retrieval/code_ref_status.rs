//! Freshness classification for a pinned code reference.
//!
//! Compares a `code_ref`'s save-time blob (`pinned_blob`) against the current
//! HEAD-tree blob and, for symbol refs, the live `code_symbols` index. Pure:
//! all git / DB lookups happen at the call site and arrive via [`CurrentRef`],
//! so every branch is unit-testable without a repo or database. File refs are
//! index-independent; symbol-ghost is index-dependent and degrades to
//! [`RefStatus::Unknown`] when no current index covers the file. See
//! [`classify`] for the exact resolution order.

/// Freshness verdict for a pinned code reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefStatus {
    /// Pinned blob equals the current HEAD-tree blob.
    Fresh,
    /// Pinned blob differs from the current HEAD-tree blob (committed code changed).
    Stale,
    /// Target no longer exists (file gone from HEAD tree, or symbol absent from a current index).
    Ghost,
    /// No anchor was captured at save (`pinned_blob` NULL).
    Unpinned,
    /// Pinned, but unverifiable now (repo not on disk, or symbol-ghost needs an absent/stale index).
    Unknown,
}

impl RefStatus {
    /// Stable serialization token (`"fresh"`/`"stale"`/`"ghost"`/`"unpinned"`/`"unknown"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            RefStatus::Fresh => "fresh",
            RefStatus::Stale => "stale",
            RefStatus::Ghost => "ghost",
            RefStatus::Unpinned => "unpinned",
            RefStatus::Unknown => "unknown",
        }
    }
}

/// The current observed state of a reference target, gathered at fetch time.
pub struct CurrentRef<'a> {
    /// Blob OID of the file in the current HEAD tree; `None` when the file is
    /// gone from HEAD or the repo could not be read.
    pub head_blob: Option<&'a str>,
    /// Whether the repo's working-tree root resolved on disk
    /// (`repo_marker.root_path` present and usable).
    pub repo_on_disk: bool,
    /// `Some(true/false)` when a current index covers the file (symbol present
    /// or absent); `None` when no current index can decide ghost-ness.
    pub symbol_present: Option<bool>,
}

/// Classify a code reference per the module's resolution order. `is_symbol`
/// selects the symbol vs. file rules; everything observable is read from `cur`.
pub fn classify(pinned_blob: Option<&str>, cur: &CurrentRef<'_>, is_symbol: bool) -> RefStatus {
    let Some(pinned) = pinned_blob else {
        return RefStatus::Unpinned;
    };
    if !cur.repo_on_disk {
        return RefStatus::Unknown;
    }
    if is_symbol {
        classify_symbol(pinned, cur)
    } else {
        classify_file(pinned, cur.head_blob)
    }
}

/// File-ref rules: a missing HEAD blob is a ghost, otherwise blob-equality
/// decides fresh vs. stale. Index-independent.
fn classify_file(pinned: &str, head_blob: Option<&str>) -> RefStatus {
    match head_blob {
        None => RefStatus::Ghost,
        Some(cur) if cur == pinned => RefStatus::Fresh,
        Some(_) => RefStatus::Stale,
    }
}

/// Symbol-ref rules: file gone -> ghost; symbol absent from a current index ->
/// ghost; no current index -> unknown; else blob-equality decides fresh/stale.
fn classify_symbol(pinned: &str, cur: &CurrentRef<'_>) -> RefStatus {
    match cur.head_blob {
        None => RefStatus::Ghost,
        Some(head) => match cur.symbol_present {
            Some(false) => RefStatus::Ghost,
            None => RefStatus::Unknown,
            Some(true) if head == pinned => RefStatus::Fresh,
            Some(true) => RefStatus::Stale,
        },
    }
}
