//! Code-graph materialization: the `index-code` post-pass that turns git
//! history and import statements into `edges` rows and a projected
//! PageRank score on every `code_symbols` row.
//!
//! One entry point, [`materialize`], runs after the symbol walk has
//! committed. The caller treats any error here as best-effort
//! (`tracing::warn!` + continue): per spec §6.3 the symbol index must
//! land even when graph materialization cannot — a broken git history or
//! a locked db never costs the user their freshly-indexed symbols.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::graph::edges::{self, EdgeKey};
use crate::graph::{cochange, imports, pagerank};
use crate::prelude::*;

/// Graph node id for a file: `file:<repo>:<path>` (the convention pinned
/// in `src/store/sql/0002_v2_tables.sql`).
fn file_node_id(repo: &str, path: &str) -> String {
    format!("file:{repo}:{path}")
}

/// Materialize the code graph for `repo`: mine new co-change pairs
/// (incremental via the `repo_marker.last_mined_commit` cursor),
/// refresh import edges for the files indexed this run, recompute
/// PageRank over the union graph, and project scores onto
/// `code_symbols.rank_score`. Runs in one transaction; the caller
/// treats failures as best-effort (the symbol index must land even
/// when graph materialization cannot).
///
/// `imports_by_file` maps repo-relative paths to the RAW module strings
/// extracted during the walk — only files actually (re)indexed this run
/// appear, so files skipped by the blob-OID gate keep their existing
/// import edges (unchanged file, unchanged imports). An empty `Vec`
/// still clears the file's stale outgoing `imports` edges: imports are
/// state, not accumulation.
///
/// When `repo` has no `code_symbols` rows yet, the call is a no-op and
/// the mining cursor is NOT advanced — advancing it before any file is
/// indexed would permanently skip the history those files appear in.
pub fn materialize(
    conn: &mut Connection,
    repo_root: &Path,
    repo: &str,
    imports_by_file: &BTreeMap<String, Vec<String>>,
) -> Result<()> {
    let tx = conn.transaction()?;
    // Sorted for the deterministic dense-index mapping PageRank needs;
    // chunk children share their parent's path so plain DISTINCT covers
    // parents and chunks alike.
    let known = known_paths(&tx, repo)?;
    if known.is_empty() {
        return Ok(());
    }

    mine_into_edges(&tx, repo_root, repo, &known)?;
    refresh_import_edges(&tx, repo, &known, imports_by_file)?;
    project_pagerank(&tx, repo, &known)?;
    tx.commit()?;
    Ok(())
}

/// Every distinct indexed path for `repo`, sorted ascending.
fn known_paths(tx: &Transaction<'_>, repo: &str) -> Result<Vec<String>> {
    let mut stmt =
        tx.prepare("SELECT DISTINCT path FROM code_symbols WHERE repo = ?1 ORDER BY path")?;
    let rows = stmt
        .query_map([repo], |r| r.get(0))?
        .collect::<std::result::Result<Vec<String>, _>>()?;
    Ok(rows)
}

/// Mine co-change pairs from commits newer than the stored cursor,
/// accumulate them onto `co_changed` edges (canonical a < b order, as
/// produced by [`cochange::mine_cochange`]), and advance the cursor to
/// HEAD. The `repo_marker` row is created on first mine; `last_head` /
/// `last_indexed_at` are preserved via the targeted `DO UPDATE`.
///
/// When the miner reports `cursor_lost` (the stored cursor's commit no
/// longer resolves — history rewrite + gc, or a corrupted marker), the
/// repo's accumulated `co_changed` edges are DELETED before the fresh
/// pairs are applied: the bounded re-mine re-counted history that earlier
/// runs already accumulated, so adding the new counts on top of the old
/// weights would double-count every surviving pair.
fn mine_into_edges(
    tx: &Transaction<'_>,
    repo_root: &Path,
    repo: &str,
    known: &[String],
) -> Result<()> {
    let cursor: Option<String> = tx
        .query_row(
            "SELECT last_mined_commit FROM repo_marker WHERE repo = ?1",
            [repo],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    let known_set: HashSet<String> = known.iter().cloned().collect();
    let outcome = cochange::mine_cochange(repo_root, &known_set, cursor.as_deref())?;
    if outcome.cursor_lost {
        // Prefix-match via substr (not LIKE) so a repo label containing
        // `%`/`_` cannot widen the delete. Both endpoints of a co_changed
        // edge live in the same repo, so matching src_id suffices.
        let prefix = format!("file:{repo}:");
        tx.execute(
            "DELETE FROM edges \
              WHERE rel = 'co_changed' AND src_kind = 'file' \
                AND substr(src_id, 1, length(?1)) = ?1",
            [&prefix],
        )?;
    }
    for pair in &outcome.pairs {
        let src = file_node_id(repo, &pair.a);
        let dst = file_node_id(repo, &pair.b);
        edges::insert_weighted(
            tx,
            EdgeKey {
                src_kind: "file",
                src_id: &src,
                dst_kind: "file",
                dst_id: &dst,
                rel: "co_changed",
            },
            i64::from(pair.count),
        )?;
    }
    tx.execute(
        "INSERT INTO repo_marker(repo, last_mined_commit) VALUES(?1, ?2) \
         ON CONFLICT(repo) DO UPDATE SET last_mined_commit = excluded.last_mined_commit",
        params![repo, outcome.cursor],
    )?;
    Ok(())
}

/// Replace the outgoing `imports` edges of every file (re)indexed this
/// run: delete the file's previous edges, then re-insert one edge per
/// raw module that [`imports::resolve`] maps unambiguously onto a known
/// path. Self-imports (a module resolving back onto its own file, e.g. a
/// stray `mod b;` inside `b.rs`) are skipped — a self-loop carries no
/// coupling signal and would only inflate the file's own PageRank.
fn refresh_import_edges(
    tx: &Transaction<'_>,
    repo: &str,
    known: &[String],
    imports_by_file: &BTreeMap<String, Vec<String>>,
) -> Result<()> {
    for (file, modules) in imports_by_file {
        let src = file_node_id(repo, file);
        tx.execute(
            "DELETE FROM edges WHERE src_kind='file' AND src_id = ?1 AND rel='imports'",
            [&src],
        )?;
        for module in modules {
            let Some(target) = imports::resolve(module, known, Some(file)) else {
                continue;
            };
            if &target == file {
                continue;
            }
            let dst = file_node_id(repo, &target);
            edges::insert(
                tx,
                EdgeKey {
                    src_kind: "file",
                    src_id: &src,
                    dst_kind: "file",
                    dst_id: &dst,
                    rel: "imports",
                },
            )?;
        }
    }
    Ok(())
}

/// Recompute PageRank over the repo's union graph (`co_changed` +
/// `imports`) and project each file's score onto every `code_symbols`
/// row sharing its path. `co_changed` edges are undirected in storage
/// (one canonical row) and expand to two directed edges here; `imports`
/// edges stay directed as stored. Edges referencing paths no longer in
/// `code_symbols` (deleted files) are skipped.
fn project_pagerank(tx: &Transaction<'_>, repo: &str, known: &[String]) -> Result<()> {
    let index: BTreeMap<&str, u32> = known
        .iter()
        .enumerate()
        .map(|(i, p)| (p.as_str(), i as u32))
        .collect();
    let prefix = format!("file:{repo}:");
    // ORDER BY makes the edge list — and therefore pagerank's f64
    // accumulation order — a function of the logical graph, not rowid
    // insertion order (an imports delete+reinsert would otherwise
    // reorder rows and perturb scores in the last ulps). The substr
    // predicate (injection-proof, same shape as the cursor-lost delete in
    // [`mine_into_edges`]) keeps other repos' rows out of the fetch; the
    // Rust-side strip below still guards the dst side and yields the
    // repo-relative paths.
    let mut stmt = tx.prepare(
        "SELECT src_id, dst_id, rel, weight FROM edges \
          WHERE rel IN ('co_changed','imports') \
            AND substr(src_id, 1, length(?1)) = ?1 \
          ORDER BY rel, src_id, dst_id",
    )?;
    let rows = stmt
        .query_map([&prefix], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut graph: Vec<(u32, u32, f64)> = Vec::new();
    for (src, dst, rel, weight) in &rows {
        // src is pre-filtered in SQL; the strip also re-checks dst (a
        // cross-repo edge must not slip in) and drops the prefix.
        let (Some(s), Some(d)) = (src.strip_prefix(&prefix), dst.strip_prefix(&prefix)) else {
            continue;
        };
        let (Some(&si), Some(&di)) = (index.get(s), index.get(d)) else {
            tracing::debug!(src = %src, dst = %dst, "materialize: edge references unindexed path; skipping");
            continue;
        };
        let w = *weight as f64;
        graph.push((si, di, w));
        if rel == "co_changed" {
            graph.push((di, si, w));
        }
    }
    let scores = pagerank::pagerank(known.len(), &graph);
    let mut update =
        tx.prepare("UPDATE code_symbols SET rank_score = ?1 WHERE repo = ?2 AND path = ?3")?;
    for (path, score) in known.iter().zip(&scores) {
        update.execute(params![score, repo, path])?;
    }
    Ok(())
}
