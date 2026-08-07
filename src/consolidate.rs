//! `comemory consolidate` — the advisory near-duplicate cluster report.
//!
//! Read-only by construction: this module scans stored SimHashes, groups the
//! live corpus into near-duplicate clusters and names the member worth
//! keeping. It never writes. Merging stays a human act
//! (`comemory save … --supersedes <id>`).

/// Union-find grouping of live fingerprints within a Hamming radius.
pub mod cluster;
/// Keeper ordering, member metadata, and in-cluster supersede resolution.
pub mod keeper;

use rusqlite::Connection;

use crate::prelude::*;
use crate::store::simhash_scan::live_simhashes;

/// One member of a near-duplicate cluster, with the stats behind the order.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Member {
    /// 8-hex memory id.
    pub id: String,
    /// Owning repo, absent for memories saved outside one.
    pub repo: Option<String>,
    /// Memory kind (`decision`, `bug`, …).
    pub kind: String,
    /// Curator quality, 1..=5.
    pub quality: u8,
    /// Lifetime retrieval hits.
    pub access_count: i64,
    /// RFC3339 instant of the last hit.
    pub last_accessed: Option<String>,
    /// Materialized memory PageRank.
    pub rank_score: f64,
    /// Hamming distance to the cluster keeper (`0` for the keeper itself).
    pub hamming_to_keeper: u32,
    /// Live member of this cluster that already supersedes this one.
    pub superseded_by: Option<String>,
}

/// A transitively-linked group of near-duplicate memories, keeper first.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Cluster {
    /// Members in keeper order — index 0 is the suggested keeper.
    pub members: Vec<Member>,
    /// Widest real pairwise distance — exposes union-find chaining.
    pub max_hamming: u32,
    /// Every member but one is already superseded from inside this cluster.
    pub resolved: bool,
}

/// What one `detect` pass should look at.
#[derive(Debug, Clone)]
pub struct Options {
    /// Hamming radius linking two fingerprints.
    pub radius: u32,
    /// Restrict the scan to one repo.
    pub repo: Option<String>,
    /// Keep clusters the operator already resolved with supersede edges.
    pub include_resolved: bool,
}

/// One `detect` pass: the clusters plus the counts the report header needs.
#[derive(Debug, Clone)]
pub struct Scan {
    /// Clusters in report order, keeper first within each.
    pub clusters: Vec<Cluster>,
    /// Live memories that carried a real fingerprint and were compared.
    pub scanned: usize,
    /// Live rows dropped because their `simhash` was never backfilled.
    pub skipped_unhashed: usize,
}

impl Scan {
    /// Memories that landed in a reported cluster.
    pub fn clustered(&self) -> usize {
        self.clusters.iter().map(|c| c.members.len()).sum()
    }
}

/// Scan live memories and group the near-duplicates. Read-only: no edge, no
/// soft-delete, no markdown write happens anywhere on this path.
///
/// Clusters the operator already handled — every member but one superseded
/// from inside the cluster — are dropped unless `include_resolved` asks for
/// them, since re-reporting settled duplication is noise.
pub fn detect(conn: &Connection, opts: &Options) -> Result<Scan> {
    let rows = live_simhashes(conn, opts.repo.as_deref(), None)?;
    let grouping = cluster::group_near_dups(&rows, opts.radius);
    let mut clusters = Vec::with_capacity(grouping.groups.len());
    for group in &grouping.groups {
        let built = keeper::build(conn, group)?;
        if opts.include_resolved || !built.resolved {
            clusters.push(built);
        }
    }
    Ok(Scan {
        clusters,
        scanned: grouping.scanned,
        skipped_unhashed: grouping.skipped_unhashed,
    })
}

#[cfg(test)]
#[path = "tests/consolidate.rs"]
mod tests;
