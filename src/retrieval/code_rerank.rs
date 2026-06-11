//! Code rerank: max-normalized relevance × the four bounded priors from
//! [`crate::retrieval::code_prior`] (PageRank, ACT-R activation,
//! working-set affinity, Beta feedback), then chunk→parent coalescing.
//! Mirrors [`crate::retrieval::rerank`] (the memory side): the same
//! zip-before-filter normalization, the same one-cached-statement-per-hit
//! signals fetch, and the same deterministic `final_score`-desc / id-asc
//! ordering.
//!
//! Consumes the [`CodeRoutedHit`] list produced by
//! [`crate::retrieval::code_route`] and emits [`CodeReranked`] entries
//! whose [`CodeScoreParts`] expose every multiplicative factor for
//! explainability.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rusqlite::{Connection, OptionalExtension};
use time::OffsetDateTime;

use crate::config::Config;
use crate::prelude::*;
use crate::retrieval::code_prior::{self, Signals};
use crate::retrieval::code_route::CodeRoutedHit;
use crate::retrieval::router::Source;
use crate::retrieval::score;

/// Number of most-recent first-parent commits whose changed files are
/// folded into the working set alongside the dirty/staged paths. Five
/// commits approximate "what the developer touched this session"
/// without dragging in stale history.
pub const WORKING_SET_COMMITS: usize = 5;

/// Multiplicative factors behind a final code score. Serialized
/// verbatim into `--json` output — a stable contract, not debug info.
/// The invariant is `final_score == f64::from(relevance) * rank *
/// activation * affinity * feedback` (up to f32 rounding of the
/// normalized relevance). The four prior factors are computed by
/// [`code_prior::priors`].
#[derive(Debug, Clone, serde::Serialize)]
pub struct CodeScoreParts {
    /// Max-normalized relevance in `[0, 1]` (pool max → 1.0,
    /// within-pool ratios preserved; degenerate pools normalize to 1.0).
    pub relevance: f32,
    /// PageRank boost (post-clamp multiplier), pool-median-relative.
    pub rank: f64,
    /// ACT-R activation boost (post-clamp multiplier).
    pub activation: f64,
    /// Working-set co-change affinity boost (post-clamp multiplier).
    pub affinity: f64,
    /// Beta feedback boost (post-clamp multiplier).
    pub feedback: f64,
    /// Product of all factors.
    pub final_score: f64,
}

/// A reranked code hit with its full identity row, ready for the final
/// cut / rendering.
#[derive(Debug, Clone)]
pub struct CodeReranked {
    /// `code_symbols.id` of the result. For a coalesced chunk win this
    /// is the PARENT row's id — the parent is the feedback-able
    /// identity a `comemory feedback` call should target — while the
    /// line range below stays the winning chunk's.
    pub symbol_id: i64,
    /// Repository the symbol was indexed from.
    pub repo: String,
    /// Repo-relative file path.
    pub path: String,
    /// Qualified symbol name (the parent's name for a coalesced chunk).
    pub symbol: String,
    /// Symbol kind, e.g. `function` (the parent's kind for a coalesced
    /// chunk).
    pub kind: String,
    /// Source language, e.g. `rust`.
    pub lang: String,
    /// First line of the match (the winning chunk's, when coalesced).
    pub line_start: i64,
    /// Last line of the match (the winning chunk's, when coalesced).
    pub line_end: i64,
    /// Which retrieval branch produced the underlying candidate.
    pub source: Source,
    /// Every multiplicative factor behind `parts.final_score`.
    pub parts: CodeScoreParts,
}

/// The set of files the developer is plausibly working on right now:
/// dirty/staged/untracked paths plus everything changed in the last
/// [`WORKING_SET_COMMITS`] first-parent commits, stored as qualified
/// `file:<repo>:<path>` graph ids ready for the affinity SQL.
#[derive(Debug, Clone, Default)]
pub struct WorkingSet {
    files: Vec<String>,
}

impl WorkingSet {
    /// Qualified `file:<repo>:<path>` ids in the set (sorted, deduped).
    /// `WorkingSet::default()` is the empty set — affinity stays
    /// neutral for callers without a repo context.
    pub fn files(&self) -> &[String] {
        &self.files
    }
}

/// Best-effort working-set detection for the repo containing
/// `repo_root`: git2 statuses (dirty, staged, and untracked paths) plus
/// the files changed in the last [`WORKING_SET_COMMITS`] first-parent
/// commits. Any git error (no repo, unborn HEAD, …) degrades to the
/// empty set with a `tracing::debug` — affinity is a bonus signal,
/// never a failure mode.
pub fn working_set(repo_root: &Path, repo: &str) -> WorkingSet {
    match collect_working_paths(repo_root) {
        Ok(paths) => WorkingSet {
            files: paths
                .into_iter()
                .map(|p| format!("file:{repo}:{p}"))
                .collect(),
        },
        Err(e) => {
            tracing::debug!(
                repo_root = %repo_root.display(),
                error = %e,
                "code_rerank: working-set detection failed; affinity neutral"
            );
            WorkingSet::default()
        }
    }
}

/// Gather the raw repo-relative working-set paths: statuses first
/// (worktree + index changes, untracked files included), then the
/// first-parent diffs of the most recent [`WORKING_SET_COMMITS`]
/// commits via [`crate::graph::cochange::commit_changed_paths`].
fn collect_working_paths(repo_root: &Path) -> Result<BTreeSet<String>> {
    use crate::git_utils::map_git_err;
    let repo = git2::Repository::discover(repo_root).map_err(map_git_err)?;
    let mut out = BTreeSet::new();

    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    for entry in repo.statuses(Some(&mut opts)).map_err(map_git_err)?.iter() {
        if let Some(path) = entry.path() {
            out.insert(path.to_string());
        }
    }

    let mut walk = repo.revwalk().map_err(map_git_err)?;
    walk.simplify_first_parent().map_err(map_git_err)?;
    walk.push_head().map_err(map_git_err)?;
    for oid in walk.take(WORKING_SET_COMMITS) {
        let commit = repo
            .find_commit(oid.map_err(map_git_err)?)
            .map_err(map_git_err)?;
        out.extend(crate::graph::cochange::commit_changed_paths(
            &repo, &commit,
        )?);
    }
    Ok(out)
}

/// Rerank code candidates by multiplying the max-normalized relevance
/// with the four bounded priors, then coalescing cAST chunk rows onto
/// their parent. Sorted by descending `final_score`, ties on ascending
/// `symbol_id` so the order is fully deterministic. Hits whose
/// `code_symbols` row vanished (raced re-index delete) are silently
/// dropped.
pub fn rerank_code(
    conn: &Connection,
    cfg: &Config,
    hits: &[CodeRoutedHit],
    working_set: &WorkingSet,
) -> Result<Vec<CodeReranked>> {
    let now = OffsetDateTime::now_utc();
    // Normalize the whole candidate pool up front, then zip — pairing
    // is established before any hit is dropped below, so a vanished
    // symbol row cannot skew which norm belongs to which hit.
    let normalized: Vec<f64> =
        score::max_normalize(&hits.iter().map(|h| f64::from(h.score)).collect::<Vec<_>>());
    let mut pool: Vec<(&CodeRoutedHit, f64, Signals)> = Vec::with_capacity(hits.len());
    for (hit, norm) in hits.iter().zip(&normalized) {
        let Some(sig) = code_prior::signals(conn, hit.symbol_id)? else {
            continue;
        };
        pool.push((hit, *norm, sig));
    }

    let median = code_prior::median_file_rank(
        pool.iter()
            .map(|(_, _, s)| ((s.repo.as_str(), s.path.as_str()), s.rank_score)),
    );
    let mut affinity_cache: BTreeMap<String, f64> = BTreeMap::new();
    let mut scored = Vec::with_capacity(pool.len());
    for (hit, norm, sig) in pool {
        let pri = code_prior::priors(
            conn,
            cfg,
            now,
            &sig,
            working_set,
            median,
            &mut affinity_cache,
        )?;
        let final_score = norm * pri.final_score;
        scored.push(Scored {
            parent_id: sig.parent_id,
            row: CodeReranked {
                symbol_id: hit.symbol_id,
                repo: sig.repo,
                path: sig.path,
                symbol: sig.symbol,
                kind: sig.kind,
                lang: sig.lang,
                line_start: sig.line_start,
                line_end: sig.line_end,
                source: hit.source,
                parts: CodeScoreParts {
                    relevance: norm as f32,
                    rank: pri.rank,
                    activation: pri.activation,
                    affinity: pri.affinity,
                    feedback: pri.feedback,
                    final_score,
                },
            },
        });
    }

    let mut out = coalesce(conn, scored)?;
    // `total_cmp` keeps the comparator a total order even if an
    // upstream stage ever leaks a NaN score (see the matching note in
    // `retrieval::rerank`).
    out.sort_by(|a, b| {
        b.parts
            .final_score
            .total_cmp(&a.parts.final_score)
            .then_with(|| a.symbol_id.cmp(&b.symbol_id))
    });
    Ok(out)
}

/// A scored candidate before chunk coalescing: the output row plus the
/// `parent_id` that decides its coalescing group.
struct Scored {
    row: CodeReranked,
    parent_id: Option<i64>,
}

/// Coalesce chunk rows onto their parent: group by
/// `parent_id.unwrap_or(symbol_id)`, keep the max-`final_score` row per
/// group (ties → lower `line_start`, deterministic). When the winner is
/// a chunk, the output carries the PARENT's id, symbol, and kind — the
/// parent is the feedback-able identity — while keeping the winning
/// chunk's line range and score parts. If the parent row vanished
/// (raced re-index delete) the chunk keeps its own identity rather than
/// pointing at a missing row.
fn coalesce(conn: &Connection, scored: Vec<Scored>) -> Result<Vec<CodeReranked>> {
    let mut groups: BTreeMap<i64, Scored> = BTreeMap::new();
    for s in scored {
        let key = s.parent_id.unwrap_or(s.row.symbol_id);
        match groups.entry(key) {
            std::collections::btree_map::Entry::Vacant(v) => {
                v.insert(s);
            }
            std::collections::btree_map::Entry::Occupied(mut o) => {
                if wins_group(&s, o.get()) {
                    o.insert(s);
                }
            }
        }
    }
    let mut out = Vec::with_capacity(groups.len());
    for (key, mut s) in groups {
        if s.parent_id.is_some() {
            if let Some((symbol, kind)) = parent_identity(conn, key)? {
                s.row.symbol_id = key;
                s.row.symbol = symbol;
                s.row.kind = kind;
            }
        }
        out.push(s.row);
    }
    Ok(out)
}

/// Group-internal winner test: higher `final_score` wins; an exact tie
/// goes to the lower `line_start` so the choice is deterministic.
fn wins_group(challenger: &Scored, incumbent: &Scored) -> bool {
    match challenger
        .row
        .parts
        .final_score
        .total_cmp(&incumbent.row.parts.final_score)
    {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => challenger.row.line_start < incumbent.row.line_start,
    }
}

/// Fetch the identity columns of a chunk's parent row; `Ok(None)` when
/// the parent vanished. `prepare_cached` for the per-group loop in
/// [`coalesce`].
fn parent_identity(conn: &Connection, parent_id: i64) -> Result<Option<(String, String)>> {
    let mut stmt = conn.prepare_cached("SELECT symbol, kind FROM code_symbols WHERE id = ?1")?;
    stmt.query_row([parent_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()
        .map_err(Error::from)
}
