//! Code rerank: max-normalized relevance × four bounded priors
//! (PageRank, ACT-R activation, working-set affinity, Beta feedback),
//! then chunk→parent coalescing. Mirrors [`crate::retrieval::rerank`]
//! (the memory side): the same zip-before-filter normalization, the
//! same one-cached-statement-per-hit signals fetch, and the same
//! deterministic `final_score`-desc / id-asc ordering.
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
use crate::retrieval::code_route::CodeRoutedHit;
use crate::retrieval::router::Source;
use crate::retrieval::score;

/// Scale for the PageRank boost: `1 + RANK_SCALE·ln(1 + raw/median)`.
/// A file at the pool median maps to `1 + 0.2·ln 2 ≈ 1.14`; the clamp
/// from `cfg.rank.prior_clamp` bounds the extremes.
pub const RANK_SCALE: f64 = 0.2;

/// Scale for the working-set co-change affinity boost:
/// `1 + AFFINITY_SCALE·ln(1 + w_sum)`. Zero co-change weight maps to
/// exactly 1.0 (neutral).
pub const AFFINITY_SCALE: f64 = 0.2;

/// Number of most-recent first-parent commits whose changed files are
/// folded into the working set alongside the dirty/staged paths. Five
/// commits approximate "what the developer touched this session"
/// without dragging in stale history.
pub const WORKING_SET_COMMITS: usize = 5;

/// Multiplicative factors behind a final code score. Serialized
/// verbatim into `--json` output — a stable contract, not debug info.
/// The invariant is `final_score == f64::from(relevance) * rank *
/// activation * affinity * feedback` (up to f32 rounding of the
/// normalized relevance).
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
/// with four bounded priors, then coalescing cAST chunk rows onto their
/// parent. Sorted by descending `final_score`, ties on ascending
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
    let clamp = cfg.rank.prior_clamp;
    // Normalize the whole candidate pool up front, then zip — pairing
    // is established before any hit is dropped below, so a vanished
    // symbol row cannot skew which norm belongs to which hit.
    let normalized: Vec<f64> =
        score::max_normalize(&hits.iter().map(|h| f64::from(h.score)).collect::<Vec<_>>());
    let mut pool: Vec<(&CodeRoutedHit, f64, Signals)> = Vec::with_capacity(hits.len());
    for (hit, norm) in hits.iter().zip(&normalized) {
        let Some(sig) = code_signals(conn, hit.symbol_id)? else {
            continue;
        };
        pool.push((hit, *norm, sig));
    }

    let median = median_file_rank(&pool);
    let mut affinity_cache: BTreeMap<String, f64> = BTreeMap::new();
    let mut scored = Vec::with_capacity(pool.len());
    for (hit, norm, sig) in pool {
        let rank = rank_boost(sig.rank_score, median, clamp);
        let days = score::days_since(&sig.last_accessed, now);
        let act = score::activation(sig.access_count, days, cfg.rank.decay);
        let activation = score::activation_boost(act, clamp);
        let affinity = file_affinity(
            conn,
            working_set,
            &sig.repo,
            &sig.path,
            clamp,
            &mut affinity_cache,
        )?;
        let feedback = score::feedback_boost(score::beta_feedback(sig.used, sig.irrelevant), clamp);
        let final_score = norm * rank * activation * affinity * feedback;
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
                    rank,
                    activation,
                    affinity,
                    feedback,
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

/// Per-symbol ranking signals pulled in one query: identity columns,
/// rank/access counters, and the (optional) feedback counters with
/// `COALESCE` neutralizing absent rows.
struct Signals {
    repo: String,
    path: String,
    symbol: String,
    kind: String,
    lang: String,
    line_start: i64,
    line_end: i64,
    rank_score: f64,
    access_count: u64,
    last_accessed: String,
    parent_id: Option<i64>,
    used: u64,
    irrelevant: u64,
}

/// Fetch the ranking signals for one code symbol. Returns `Ok(None)`
/// when the row vanished (raced re-index delete). `prepare_cached` so
/// the per-hit loop in [`rerank_code`] reuses one prepared statement.
fn code_signals(conn: &Connection, symbol_id: i64) -> Result<Option<Signals>> {
    let mut stmt = conn.prepare_cached(
        "SELECT c.repo, c.path, c.symbol, c.kind, c.lang, c.line_start, c.line_end,
                c.rank_score, c.access_count, COALESCE(c.last_accessed, c.indexed_at),
                c.parent_id, COALESCE(f.used_count, 0), COALESCE(f.irrelevant_count, 0)
           FROM code_symbols c
           LEFT JOIN code_feedback f ON f.symbol_id = c.id
          WHERE c.id = ?1",
    )?;
    stmt.query_row([symbol_id], |r| {
        Ok(Signals {
            repo: r.get(0)?,
            path: r.get(1)?,
            symbol: r.get(2)?,
            kind: r.get(3)?,
            lang: r.get(4)?,
            line_start: r.get(5)?,
            line_end: r.get(6)?,
            rank_score: r.get(7)?,
            access_count: r.get::<_, i64>(8)?.max(0) as u64,
            last_accessed: r.get(9)?,
            parent_id: r.get(10)?,
            used: r.get::<_, i64>(11)?.max(0) as u64,
            irrelevant: r.get::<_, i64>(12)?.max(0) as u64,
        })
    })
    .optional()
    .map_err(Error::from)
}

/// Median of the candidate pool's DISTINCT per-file `rank_score`s
/// (chunk rows share their file's projected score, so dedup is by
/// `(repo, path)`). Absolute PageRank scales with `1/file-count`, so a
/// fixed reference would make the boost depend on repo size;
/// median-relative mapping is repo-size invariant. Even-sized pools
/// take the mean of the middle two; an empty pool returns 0.0, which
/// [`rank_boost`] treats as "unranked repo → neutral".
fn median_file_rank(pool: &[(&CodeRoutedHit, f64, Signals)]) -> f64 {
    let by_file: BTreeMap<(&str, &str), f64> = pool
        .iter()
        .map(|(_, _, s)| ((s.repo.as_str(), s.path.as_str()), s.rank_score))
        .collect();
    let mut ranks: Vec<f64> = by_file.into_values().collect();
    if ranks.is_empty() {
        return 0.0;
    }
    ranks.sort_by(f64::total_cmp);
    let n = ranks.len();
    if n % 2 == 1 {
        ranks[n / 2]
    } else {
        (ranks[n / 2 - 1] + ranks[n / 2]) / 2.0
    }
}

/// PageRank prior: `bounded(1 + RANK_SCALE·ln(1 + raw/median), clamp)`,
/// pool-median-relative (see [`median_file_rank`] for why). A
/// non-positive median (unranked repo, every `rank_score` at the 0.0
/// column default) keeps every rank prior at the neutral 1.0.
fn rank_boost(raw: f64, median: f64, clamp: (f64, f64)) -> f64 {
    if median <= 0.0 {
        return 1.0;
    }
    score::bounded_boost(1.0 + RANK_SCALE * (1.0 + raw.max(0.0) / median).ln(), clamp)
}

/// Working-set affinity prior for one candidate file:
/// `bounded(1 + AFFINITY_SCALE·ln(1 + w_sum), clamp)` where `w_sum` is
/// the total `co_changed` edge weight between the candidate's file and
/// the working-set files. Cached per distinct candidate file in
/// `cache` so a pool with many symbols from one file runs one edge
/// query. An empty working set short-circuits to neutral 1.0 with no
/// query at all.
fn file_affinity(
    conn: &Connection,
    ws: &WorkingSet,
    repo: &str,
    path: &str,
    clamp: (f64, f64),
    cache: &mut BTreeMap<String, f64>,
) -> Result<f64> {
    if ws.files.is_empty() {
        return Ok(1.0);
    }
    let fid = format!("file:{repo}:{path}");
    if let Some(boost) = cache.get(&fid) {
        return Ok(*boost);
    }
    let w_sum = co_change_weight(conn, &fid, &ws.files)?;
    let boost = score::bounded_boost(1.0 + AFFINITY_SCALE * (1.0 + w_sum).ln(), clamp);
    cache.insert(fid, boost);
    Ok(boost)
}

/// Total `co_changed` weight between `fid` and the working-set file
/// ids, in either direction (the miner stores one canonical row per
/// undirected pair). Numbered placeholders are reused across both `IN`
/// lists so the parameter vector binds once; `prepare_cached` caches
/// one statement per working-set arity.
fn co_change_weight(conn: &Connection, fid: &str, ws_files: &[String]) -> Result<f64> {
    let marks = (0..ws_files.len())
        .map(|i| format!("?{}", i + 2))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT COALESCE(SUM(weight), 0) FROM edges \
          WHERE rel = 'co_changed' AND src_kind = 'file' AND dst_kind = 'file' \
            AND ((src_id = ?1 AND dst_id IN ({marks})) \
              OR (dst_id = ?1 AND src_id IN ({marks})))"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let params =
        rusqlite::params_from_iter(std::iter::once(fid).chain(ws_files.iter().map(String::as_str)));
    let w: i64 = stmt.query_row(params, |r| r.get(0))?;
    Ok(w.max(0) as f64)
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
