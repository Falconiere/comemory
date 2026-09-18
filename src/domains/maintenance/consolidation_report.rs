//! The owned value `comemory consolidate` produces.
//!
//! `maintenance::consolidate::run` returns this so the CLI's `--json`/TTY writers and
//! the `/api/v1/consolidate` handler each build their own envelope from one
//! value. The near-duplicate capability's result model, not a rendering
//! concern (#166).

use serde::Serialize;

use crate::domains::maintenance::consolidation::Cluster;
use crate::utilities::pagination::Page;

/// The report `comemory consolidate` emits, in JSON and TTY alike.
#[derive(Debug, Serialize)]
pub struct Report {
    /// Radius the clusters were built at.
    pub radius: u32,
    /// Live memories carrying a real fingerprint that were compared.
    pub scanned: usize,
    /// Live rows skipped because their `simhash` was never backfilled.
    pub skipped_unhashed: usize,
    /// Memories that landed in a reported cluster.
    pub clustered: usize,
    /// The windowed clusters.
    pub clusters: Page<Cluster>,
}
