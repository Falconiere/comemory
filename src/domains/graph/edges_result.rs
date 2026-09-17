//! The owned value `comemory edges` produces.
//!
//! [`edges::run`](super::edges::run) returns this so the CLI writers and the `/api/v1/edges`
//! handler each build their own envelope from one value. The graph
//! capability's result model, not a rendering concern (#166).

use crate::store::edge_fts::EdgeFtsHit;

/// Everything `comemory edges`'s render layer needs: [`edges::run`](super::edges::run)
/// returns this so the CLI and the HTTP handler can each build their own
/// envelope from one owned value instead of four loose parameters.
pub struct EdgesResult {
    /// Matched triplet rows for the requested page.
    pub hits: Vec<EdgeFtsHit>,
    /// Requested page size.
    pub limit: usize,
    /// Number of leading ranked relations skipped.
    pub offset: usize,
    /// Whether more in-window ranked relations exist beyond this page.
    pub has_more: bool,
}
