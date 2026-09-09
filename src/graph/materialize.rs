//! Code-graph materialization: the `index-code` post-pass that turns git
//! history and import statements into `edges` rows and a projected
//! PageRank score on every `code_symbols` row.
//!
//! One entry point, [`materialize`], runs after the symbol walk has
//! committed. The caller treats any error here as best-effort
//! (`tracing::warn!` + continue): per spec §6.3 the symbol index must
//! land even when graph materialization cannot — a broken git history or
//! a locked db never costs the user their freshly-indexed symbols.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use crate::graph::{coactivate, cochange, imports, pagerank};
use crate::prelude::*;
use crate::store::code_row;
use crate::store::edges::{self, EdgeKey, file_node_id, file_node_prefix};
use crate::store::memory_row;
use crate::store::repo_marker;
use crate::store::{Connection, Transaction};
use time::OffsetDateTime;

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
///
/// `lookback_days` is the search→edit `retrieval_log` window passed through
/// to [`coactivate::harvest`] (from `Config.reinforce.search_edit_days`), or
/// `None` when `Config.reinforce.enabled` is off — in which case the harvest
/// is SKIPPED entirely rather than run with a zero window. The cursor still
/// advances: disabling reinforcement means "stop rewarding from here on", not
/// "replay this history once someone re-enables it".
pub fn materialize(
    conn: &mut Connection,
    repo_root: &Path,
    repo: &str,
    imports_by_file: &BTreeMap<String, Vec<String>>,
    lookback_days: Option<u32>,
) -> Result<()> {
    let tx = conn.transaction()?;
    // Sorted for the deterministic dense-index mapping PageRank needs;
    // chunk children share their parent's path so plain DISTINCT covers
    // parents and chunks alike.
    let known = known_paths(&tx, repo)?;
    if known.is_empty() {
        return Ok(());
    }

    let (touched, cursor) = mine_into_edges(&tx, repo_root, repo, &known)?;
    // Built once per run: per-module resolution against the index replaces
    // the per-call rescan of every known path.
    let index = imports::PathIndex::new(&known);
    refresh_import_edges(&tx, repo, &index, imports_by_file)?;
    recompute_rank(&tx, repo)?;
    // Co-activation reward: AFTER pagerank, BEFORE the cursor advances —
    // so the reinforcement is atomic with the cursor (a crash can't
    // half-apply it, and the cursor still reflects only-harvested-once).
    if let Some(days) = lookback_days {
        let at = memory_row::iso_format(OffsetDateTime::now_utc())?;
        coactivate::harvest(&tx, repo, &touched, &at, days)?;
    }
    advance_cursor(&tx, repo, &cursor)?;
    tx.commit()?;
    Ok(())
}

/// Every distinct indexed path for `repo`, sorted ascending.
fn known_paths(tx: &Transaction<'_>, repo: &str) -> Result<Vec<String>> {
    code_row::distinct_paths_for_repo(tx, repo)
}

/// Mine co-change pairs from commits newer than the stored cursor and
/// accumulate them onto `co_changed` edges (canonical a < b order, as
/// produced by [`cochange::mine_cochange`]). Returns the per-pass touch map
/// and the new HEAD cursor for the caller to feed the co-activation reward
/// and then [`advance_cursor`] — the cursor is NOT written here.
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
) -> Result<(HashMap<String, u32>, String)> {
    let cursor = repo_marker::last_mined_commit(tx, repo)?;
    let known_set: HashSet<String> = known.iter().cloned().collect();
    let outcome = cochange::mine_cochange(repo_root, &known_set, cursor.as_deref())?;
    if outcome.cursor_lost {
        // Prefix-match via substr (not LIKE) so a repo label containing
        // `%`/`_` cannot widen the delete. Both endpoints of a co_changed
        // edge live in the same repo, so matching src_id suffices.
        let prefix = file_node_prefix(repo);
        edges::delete_co_changed_for_repo(tx, &prefix)?;
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
    // The cursor is NOT advanced here: the co-activation reward must run
    // (and commit atomically) before the marker moves, so a crash can't
    // leave commits credited-but-cursored or cursored-but-uncredited. The
    // touch map (all changed paths this pass, not just `known_files`) feeds
    // that reward.
    Ok((outcome.touched, outcome.cursor))
}

/// Advance the `repo_marker.last_mined_commit` cursor to the HEAD oid the
/// miner reported. Split out of [`mine_into_edges`] so it runs AFTER the
/// co-activation reward within the same transaction: the cursor and the
/// reward commit together, keeping the once-only harvest invariant intact
/// even on a crash. The `repo_marker` row is created on first mine;
/// `last_head` / `last_indexed_at` are preserved via the targeted update.
fn advance_cursor(tx: &Transaction<'_>, repo: &str, cursor: &str) -> Result<()> {
    repo_marker::advance_mined_cursor(tx, repo, cursor)
}

/// Replace the outgoing `imports` edges of every file (re)indexed this
/// run: delete the file's previous edges, then re-insert one edge per
/// raw module that [`imports::PathIndex::resolve`] maps unambiguously
/// onto a known path. Self-imports (a module resolving back onto its own
/// file, e.g. a stray `mod b;` inside `b.rs`) are skipped — a self-loop
/// carries no coupling signal and would only inflate the file's own
/// PageRank.
fn refresh_import_edges(
    tx: &Transaction<'_>,
    repo: &str,
    index: &imports::PathIndex,
    imports_by_file: &BTreeMap<String, Vec<String>>,
) -> Result<()> {
    for (file, modules) in imports_by_file {
        let src = file_node_id(repo, file);
        edges::delete_imports_from(tx, &src)?;
        for module in modules {
            let Some(target) = index.resolve(module, Some(file)) else {
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

/// Recompute PageRank for `repo` over the edges ALREADY stored (no mining,
/// no import refresh) and re-project it onto `code_symbols.rank_score`,
/// returning the number of `code_symbols` rows written.
///
/// The tail of [`materialize`], factored out so `POST /api/v1/graph/recompute`
/// can re-derive the ranks of every repo from the existing graph without
/// touching git history — both callers therefore run the identical
/// projection (Binding Rule 1). A repo with no indexed files is a no-op.
pub(crate) fn recompute_rank(tx: &Transaction<'_>, repo: &str) -> Result<u64> {
    let known = known_paths(tx, repo)?;
    if known.is_empty() {
        return Ok(0);
    }
    project_pagerank(tx, repo, &known)
}

/// Recompute PageRank over the repo's union graph (`co_changed` +
/// `imports`) and project each file's score onto every `code_symbols`
/// row sharing its path. `co_changed` edges are undirected in storage
/// (one canonical row) and expand to two directed edges here; `imports`
/// edges stay directed as stored. Edges referencing paths no longer in
/// `code_symbols` (deleted files) are skipped. Returns the number of rows
/// the projection updated.
fn project_pagerank(tx: &Transaction<'_>, repo: &str, known: &[String]) -> Result<u64> {
    let index: BTreeMap<&str, u32> = known
        .iter()
        .enumerate()
        .map(|(i, p)| (p.as_str(), i as u32))
        .collect();
    let prefix = file_node_prefix(repo);
    // ORDER BY makes the edge list — and therefore pagerank's f64
    // accumulation order — a function of the logical graph, not rowid
    // insertion order (an imports delete+reinsert would otherwise
    // reorder rows and perturb scores in the last ulps). The substr
    // predicate (injection-proof, same shape as the cursor-lost delete in
    // [`mine_into_edges`]) keeps other repos' rows out of the fetch; the
    // Rust-side strip below still guards the dst side and yields the
    // repo-relative paths.
    let rows = edges::co_changed_and_imports_edges(tx, &prefix)?;
    let mut graph: Vec<(u32, u32, f64)> = Vec::new();
    for row in &rows {
        // src is pre-filtered in SQL; the strip also re-checks dst (a
        // cross-repo edge must not slip in) and drops the prefix.
        let (Some(s), Some(d)) = (
            row.src_id.strip_prefix(&prefix),
            row.dst_id.strip_prefix(&prefix),
        ) else {
            continue;
        };
        let (Some(&si), Some(&di)) = (index.get(s), index.get(d)) else {
            tracing::debug!(
                src = %row.src_id, dst = %row.dst_id,
                "materialize: edge references unindexed path; skipping"
            );
            continue;
        };
        let w = row.weight as f64;
        graph.push((si, di, w));
        if row.rel == "co_changed" {
            graph.push((di, si, w));
        }
    }
    let scores = pagerank::pagerank(known.len(), &graph);
    code_row::update_rank_scores(tx, repo, known, &scores)
}

#[cfg(test)]
#[path = "tests/materialize.rs"]
mod tests;
