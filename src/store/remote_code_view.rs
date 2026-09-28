//! Reads over the pulled projection that answer a question about the repo
//! rather than about one generation.
//!
//! [`super::remote_code`] reads a generation by id — what a named generation
//! holds. These read the ACTIVE generation instead: which repos this machine
//! knows only because a peer shared them, and what the shared side
//! contributes to the code graph. They are split from `remote_code` because
//! their scope is the repo, and because that file is near the size ceiling.

use rusqlite::Connection;

use crate::prelude::*;
use crate::store::edges::{GraphEdgeRow, file_node_prefix};

/// What a peer has shared about one repo: the generation that is active here
/// and the revision it describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedRepo {
    /// Repo label, as the sender filed it.
    pub repo: String,
    /// The active generation's id.
    pub generation_id: String,
    /// The head the sender indexed at.
    pub head: String,
    /// The commit its co-change edges were mined through, when it mined any.
    pub mined_commit: Option<String>,
    /// Files in the shared manifest.
    pub file_count: i64,
    /// When this machine activated it.
    pub activated_at: Option<String>,
}

/// Every repo whose active generation came from a peer, ascending by label —
/// never one this machine built itself — and, once a policy is loaded, never
/// a repo `repository_approval` currently withholds (hidden, not deleted;
/// #256 B-7). A hub or bare `serve` has no snapshot and lists every share.
///
/// # Errors
/// Propagates SQLite failures.
pub fn shared_repos(conn: &Connection) -> Result<Vec<SharedRepo>> {
    let mut statement = conn.prepare(
        "SELECT repo, generation_id, head, mined_commit, file_count, activated_at \
           FROM code_generation \
          WHERE state = 'active' AND origin = 'sync' \
            AND (NOT EXISTS (SELECT 1 FROM sync_policy_snapshot) \
                 OR repo IN (SELECT canonical FROM repository_approval)) \
          ORDER BY repo",
    )?;
    let rows = statement
        .query_map([], |r| {
            Ok(SharedRepo {
                repo: r.get(0)?,
                generation_id: r.get(1)?,
                head: r.get(2)?,
                mined_commit: r.get(3)?,
                file_count: r.get(4)?,
                activated_at: r.get(5)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Symbols in the active generation of every repo [`shared_repos`] reports,
/// optionally scoped to one repo — the pulled half of `stats`'s
/// `code_symbols`. Only a peer's generation counts: a locally built one is
/// already in `code_symbols`, and a repo has one active generation at a time,
/// so nothing is counted twice.
///
/// # Errors
/// Propagates SQLite failures.
pub fn shared_symbol_count(conn: &Connection, repo: Option<&str>) -> Result<u64> {
    let scope = if repo.is_some() {
        " AND s.repo = ?1"
    } else {
        ""
    };
    let sql = format!(
        "SELECT COUNT(*) FROM remote_code_symbol s \
           JOIN code_generation g \
             ON g.repo = s.repo AND g.generation_id = s.generation_id \
          WHERE g.state = 'active' AND g.origin = 'sync' \
            AND (NOT EXISTS (SELECT 1 FROM sync_policy_snapshot) \
                 OR s.repo IN (SELECT canonical FROM repository_approval)){scope}"
    );
    let count: i64 = match repo {
        Some(repo) => conn.query_row(&sql, [repo], |r| r.get(0))?,
        None => conn.query_row(&sql, [], |r| r.get(0))?,
    };
    Ok(count.max(0) as u64)
}

/// The edges the shared side contributes, as file node ids, optionally scoped
/// to one repo.
///
/// This is [`SHARED_EDGES`] read on its own: the same rows
/// [`super::code_graph_edges::fetch_page`] unions into its window, so a
/// caller asking which side an edge came from and a caller paging the whole
/// graph cannot disagree.
///
/// # Errors
/// Propagates SQLite failures.
pub fn shared_edges(conn: &Connection, repo: Option<&str>) -> Result<Vec<GraphEdgeRow>> {
    let scope = if repo.is_some() {
        " WHERE substr(src_id, 1, length(?1)) = ?1"
    } else {
        ""
    };
    let sql = format!(
        "SELECT src_id, dst_id, rel, weight FROM ({SHARED_EDGES}){scope} \
          ORDER BY weight DESC, rel ASC, src_id ASC, dst_id ASC"
    );
    let mut statement = conn.prepare(&sql)?;
    let map = |r: &rusqlite::Row<'_>| {
        Ok(GraphEdgeRow {
            src_id: r.get(0)?,
            dst_id: r.get(1)?,
            rel: r.get(2)?,
            weight: r.get(3)?,
        })
    };
    let rows: std::result::Result<Vec<_>, _> = match repo {
        Some(repo) => statement
            .query_map([file_node_prefix(repo)], map)?
            .collect(),
        None => statement.query_map([], map)?.collect(),
    };
    Ok(rows?)
}

/// The shared half of the code graph: every ACTIVE generation's edges, keyed
/// like a local edge (`file:<repo>:<path>`), gated by policy exactly as
/// [`shared_repos`] is, and with a local edge for the same pair left out so
/// the pair appears once, at the local weight. Both the direct read above and
/// the paginated window in [`super::code_graph_edges::fetch_page`] select
/// from this one definition, so neither can disagree with the other.
pub(crate) const SHARED_EDGES: &str = "SELECT \
         'file:' || e.repo || ':' || e.src_path AS src_id, \
         'file:' || e.repo || ':' || e.dst_path AS dst_id, \
         e.rel AS rel, e.weight AS weight \
       FROM remote_code_edge e \
       JOIN code_generation g \
         ON g.repo = e.repo AND g.generation_id = e.generation_id \
      WHERE g.state = 'active' \
        AND (NOT EXISTS (SELECT 1 FROM sync_policy_snapshot) \
             OR e.repo IN (SELECT canonical FROM repository_approval)) \
        AND NOT EXISTS (SELECT 1 FROM edges l \
                         WHERE l.src_kind = 'file' \
                           AND l.src_id = 'file:' || e.repo || ':' || e.src_path \
                           AND l.dst_kind = 'file' \
                           AND l.dst_id = 'file:' || e.repo || ':' || e.dst_path \
                           AND l.rel = e.rel)";

#[cfg(test)]
#[path = "tests/remote_code_view.rs"]
mod tests;
