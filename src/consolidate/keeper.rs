//! Keeper ordering, member metadata, and in-cluster supersede resolution.
//!
//! The keeper rule reuses the priors the ranker already trusts rather than
//! inventing a score: quality, then how often the memory is actually
//! retrieved, then its materialized PageRank, then the id as a total order.

use std::collections::HashMap;

use super::{Cluster, Member};
use crate::consolidate::cluster::Group;
use crate::prelude::*;
use crate::simhash::hamming64;
use crate::store::Connection;

/// Per-memory stats behind the keeper order, re-exported from
/// [`crate::store::memory_meta`] so this module's field accesses are
/// unchanged. `Default` is derived there for [`keeper_order`]'s
/// vanished-row fallback.
type Stats = crate::store::memory_meta::KeeperStats;

/// Assemble one reported cluster: order the members, mark the keeper, and
/// resolve which of them are already superseded from inside the cluster.
pub fn build(conn: &Connection, group: &Group) -> Result<Cluster> {
    let ids: Vec<String> = group.rows.iter().map(|r| r.id.clone()).collect();
    let stats = load_stats(conn, &ids)?;
    let mut rows = group.rows.clone();
    rows.sort_by(|a, b| keeper_order(stats.get(&a.id), stats.get(&b.id), &a.id, &b.id));

    let superseders = in_cluster_superseders(conn, &ids)?;
    let keeper_hash = rows.first().map_or(0, |r| r.simhash as u64);
    let members: Vec<Member> = rows
        .iter()
        .map(|row| {
            let s = stats.get(&row.id).cloned().unwrap_or_default();
            Member {
                id: row.id.clone(),
                repo: s.repo,
                kind: s.kind,
                quality: s.quality,
                access_count: s.access_count,
                last_accessed: s.last_accessed,
                rank_score: s.rank_score,
                hamming_to_keeper: hamming64(keeper_hash, row.simhash as u64),
                superseded_by: superseders.get(&row.id).cloned(),
            }
        })
        .collect();

    let superseded = members.iter().filter(|m| m.superseded_by.is_some()).count();
    Ok(Cluster {
        resolved: !members.is_empty() && superseded + 1 >= members.len(),
        max_hamming: group.max_hamming,
        members,
    })
}

/// Order two members best-keeper-first. A member whose row vanished between
/// the scan and this load sorts on neutral defaults rather than failing the
/// whole report.
fn keeper_order(
    a: Option<&Stats>,
    b: Option<&Stats>,
    a_id: &str,
    b_id: &str,
) -> std::cmp::Ordering {
    let (x, y) = (
        a.cloned().unwrap_or_default(),
        b.cloned().unwrap_or_default(),
    );
    y.quality
        .cmp(&x.quality)
        .then(y.access_count.cmp(&x.access_count))
        .then(y.last_accessed.cmp(&x.last_accessed))
        .then(y.rank_score.total_cmp(&x.rank_score))
        .then(a_id.cmp(b_id))
}

/// Batch-load the keeper-order stats for exactly the clustered ids.
fn load_stats(conn: &Connection, ids: &[String]) -> Result<HashMap<String, Stats>> {
    let borrowed: Vec<&str> = ids.iter().map(String::as_str).collect();
    let rows = crate::store::memory_meta::keeper_stats(conn, &borrowed)?;
    Ok(rows.into_iter().collect())
}

/// Map each member to the live cluster member that supersedes it, if any.
///
/// Resolution is judged inside the cluster, never against the keeper:
/// supersede edges point forward in time while the keeper rule is
/// quality-first, so a chain that ends on a non-keeper is still a handled
/// cluster. Ties pick the lowest superseder id so the report is stable.
fn in_cluster_superseders(conn: &Connection, ids: &[String]) -> Result<HashMap<String, String>> {
    let members: std::collections::HashSet<&str> = ids.iter().map(String::as_str).collect();
    let depth = ids.len() as u32;
    let mut out: HashMap<String, String> = HashMap::new();
    for src in ids {
        for dst in crate::store::edges::supersedes_chain(conn, src, depth)? {
            if dst == *src || !members.contains(dst.as_str()) {
                continue;
            }
            out.entry(dst)
                .and_modify(|cur| {
                    if src < cur {
                        cur.clone_from(src);
                    }
                })
                .or_insert_with(|| src.clone());
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "tests/keeper.rs"]
mod tests;
