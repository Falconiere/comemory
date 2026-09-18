//! The owned value `comemory prune` produces.
//!
//! `maintenance::prune::run` returns this so the CLI's `--json`/TTY writers and the
//! `/api/v1/prune` handler each build their own envelope from one value. The
//! retention capability's result model, not a rendering concern (#166).

use serde::Serialize;

use crate::utilities::pagination::Page;

/// Output schema for both JSON and TTY rendering.
#[derive(Serialize, Debug)]
pub struct Report {
    /// Count of `edges` rows whose source memory is missing or
    /// soft-deleted. A bare count (already a number, not a list), so it
    /// is never paginated.
    pub orphan_edges: i64,
    /// Paginated `<repo>:<path>` values whose corresponding `indexed_files`
    /// row has been removed. The repo prefix disambiguates identical paths
    /// across different repos (e.g. `src/main.rs` in two checkouts). The
    /// shared `--limit` / `--offset` window applies to the dry-run display
    /// only.
    pub stale_code_files: Page<String>,
    /// Paginated rows flagged by [`crate::domains::maintenance::retention::low_value::detect_with_reasons`]
    /// — soft-delete candidates (applied to the FULL set, not the page, when
    /// `apply` is set). The window applies to the dry-run display only.
    pub low_value_memories: Page<PruneRow>,
    /// Paginated rows flagged by [`crate::domains::maintenance::retention::stale_code::detect`] —
    /// owners of a pinned `references_symbol` whose target is a `ghost`
    /// (gone from a current index). Advisory: surfaced for the operator,
    /// never deleted by `apply` (spec Non-Goal 5). The window applies to
    /// display only.
    pub ghost_ref_memories: Page<PruneRow>,
    /// Count of files under `memories/.trash/` right now (corpus-level,
    /// unwindowed — every trashed file, not just this page's candidates).
    pub trash_count: u64,
    /// Summed size, in bytes, of every file under `memories/.trash/` —
    /// what a `comemory gc` run right now would be able to reclaim.
    pub reclaimable_bytes: u64,
    /// An `--apply` run soft-deleted memories but could not rebuild
    /// `edge_fts` afterwards, so relation search is behind until the next
    /// write refreshes it. The deletes themselves committed — this is a
    /// freshness warning, not a failure — and it is always false for a
    /// dry run. Omitted from the JSON when false, as in `gc`'s report.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub derived_stale: bool,
}

/// One prune candidate, enriched for the console: which memory, why it was
/// flagged, and the two signals an operator uses to judge the call.
#[derive(Serialize, Debug)]
pub struct PruneRow {
    /// The flagged memory's id.
    pub id: String,
    /// First non-empty trimmed line of the body ([`crate::domains::memories::nav::title_of`]).
    pub title: String,
    /// Which detector flagged this row: `"low value"`, `"orphan"`, or
    /// `"stale code"`.
    pub reason: String,
    /// ACT-R activation ([`crate::domains::retrieval::score::activation`]) at scan
    /// time, using the configured decay.
    pub activation: f64,
    /// Whole days since the memory was created.
    pub age_days: u64,
}
