//! The single post-write pass that refreshes every derived artifact.
//!
//! Two artifacts are computed from `memories` + `edges` rather than written
//! incrementally: `memories.rank_score` ([`crate::graph::memory_rank`]) and
//! the `edge_fts` triplet index ([`crate::store::edge_fts`]). They share
//! one staleness window and one set of trigger seams, so they share one
//! entry point — a new seam cannot refresh half the derived state.

use rusqlite::Connection;

use crate::graph::memory_rank;
use crate::store::edge_fts;

/// Refresh both derived artifacts after a write to `memories` or `edges`.
///
/// Rank runs first, then the triplet index — the order the seams read in,
/// and the order that leaves `edge_fts` reflecting the same graph the
/// scores were computed over. Each artifact is independently best-effort:
/// a failure warns and the other still runs. Callers invoke this *after*
/// their own transaction has committed, so a failed refresh costs only
/// freshness, never the primary write.
pub fn refresh_derived_best_effort(conn: &mut Connection) {
    memory_rank::refresh_best_effort(conn);
    if let Err(e) = edge_fts::refresh(conn) {
        tracing::warn!(error = %e, "edge_fts: refresh failed; triplet index left stale");
    }
}

#[cfg(test)]
#[path = "tests/derived.rs"]
mod tests;
