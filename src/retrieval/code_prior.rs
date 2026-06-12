//! The four bounded code priors — PageRank, ACT-R activation, working-set
//! co-change affinity, Beta feedback — computed in exactly one place.
//!
//! Two consumers share this math: [`crate::retrieval::code_rerank`]
//! multiplies the prior product into a max-normalized relevance score for
//! `comemory search-code`, and [`crate::retrieval::bundle`] ranks the code
//! refs of `comemory context` by the prior product alone — refs are
//! address-resolved by the graph walk, not query-matched, so they carry no
//! relevance term. Both follow the same pooled discipline: fetch
//! [`signals`] once per candidate, derive the median via
//! [`median_file_rank`], then score with [`priors`] under one shared clock
//! and one shared affinity cache.

use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension};
use time::OffsetDateTime;

use crate::config::Config;
use crate::prelude::*;
use crate::retrieval::code_rerank::WorkingSet;
use crate::retrieval::score;

/// Scale for the PageRank boost: `1 + RANK_SCALE·ln(1 + raw/median)`.
/// A file at the pool median maps to `1 + 0.2·ln 2 ≈ 1.14`; the clamp
/// from `cfg.rank.prior_clamp` bounds the extremes.
pub const RANK_SCALE: f64 = 0.2;

/// Scale for the working-set co-change affinity boost:
/// `1 + AFFINITY_SCALE·ln(1 + w_sum)`. Zero co-change weight maps to
/// exactly 1.0 (neutral).
pub const AFFINITY_SCALE: f64 = 0.2;

/// The four multiplicative graph priors behind a code ranking, plus their
/// product. Serialized verbatim into `--json` output (the `rank_parts`
/// object on `comemory context` code refs) — a stable contract, not debug
/// info. Unlike [`crate::retrieval::code_rerank::CodeScoreParts`] there is
/// no relevance term: the invariant is
/// `final_score == rank * activation * affinity * feedback`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CodePriorParts {
    /// PageRank boost (post-clamp multiplier), pool-median-relative.
    pub rank: f64,
    /// ACT-R activation boost (post-clamp multiplier).
    pub activation: f64,
    /// Working-set co-change affinity boost (post-clamp multiplier).
    pub affinity: f64,
    /// Beta feedback boost (post-clamp multiplier).
    pub feedback: f64,
    /// Product of the four priors.
    pub final_score: f64,
}

/// Per-symbol ranking signals pulled in one query: identity columns,
/// rank/access counters, and the (optional) feedback counters with
/// `COALESCE` neutralizing absent rows.
pub struct Signals {
    /// Repository the symbol was indexed from.
    pub repo: String,
    /// Repo-relative file path.
    pub path: String,
    /// Qualified symbol name.
    pub symbol: String,
    /// Symbol kind, e.g. `function`.
    pub kind: String,
    /// Source language, e.g. `rust`.
    pub lang: String,
    /// First line of the symbol.
    pub line_start: i64,
    /// Last line of the symbol.
    pub line_end: i64,
    /// Projected PageRank score of the symbol's file.
    pub rank_score: f64,
    /// Times the symbol was returned by a tracked search.
    pub access_count: u64,
    /// Last access timestamp (falls back to `indexed_at`).
    pub last_accessed: String,
    /// Parent `code_symbols` rowid for cAST chunk rows.
    pub parent_id: Option<i64>,
    /// `code_feedback.used_count` under the row's effective identity.
    pub used: u64,
    /// `code_feedback.irrelevant_count` under the row's effective identity.
    pub irrelevant: u64,
}

/// Fetch the ranking signals for one code symbol. Returns `Ok(None)` when
/// the row vanished (raced re-index delete). `prepare_cached` so per-hit
/// loops reuse one prepared statement.
///
/// `code_feedback` is keyed by stable (repo, path, symbol) identity (see
/// `stats::code_feedback`), joined here by the row's EFFECTIVE identity:
/// the CLI feedback path records against the COALESCED parent id, so a
/// cAST chunk row (`parent_id` NOT NULL, symbol `<name>#<n>`) never owns a
/// feedback row of its own — it inherits the PARENT's counters via the
/// `COALESCE(parent.symbol, c.symbol)` join so the parent's feedback
/// influences its chunks while they are scored pre-coalesce.
pub fn signals(conn: &Connection, symbol_id: i64) -> Result<Option<Signals>> {
    let mut stmt = conn.prepare_cached(
        "SELECT c.repo, c.path, c.symbol, c.kind, c.lang, c.line_start, c.line_end,
                c.rank_score, c.access_count, COALESCE(c.last_accessed, c.indexed_at),
                c.parent_id, COALESCE(f.used_count, 0), COALESCE(f.irrelevant_count, 0)
           FROM code_symbols c
           LEFT JOIN code_feedback f
                  ON f.repo = c.repo AND f.path = c.path
                 AND f.symbol = COALESCE(
                       (SELECT p.symbol FROM code_symbols p WHERE p.id = c.parent_id),
                       c.symbol)
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

/// Compute the four bounded priors for one signals row — the single home
/// of the prior math. Pool-scoring callers (`rerank_code`, the context
/// bundle) pass a shared `now` so one pool is judged against one clock,
/// and a shared `affinity_cache` so many symbols from one file run one
/// edge query. Derive `pool_median_rank` for the caller's candidate set
/// via [`median_file_rank`] over the pool's fetched [`signals`] rows.
pub fn priors(
    conn: &Connection,
    cfg: &Config,
    now: OffsetDateTime,
    sig: &Signals,
    working_set: &WorkingSet,
    pool_median_rank: f64,
    affinity_cache: &mut BTreeMap<String, f64>,
) -> Result<CodePriorParts> {
    let clamp = cfg.rank.prior_clamp;
    let rank = rank_boost(sig.rank_score, pool_median_rank, clamp);
    let days = score::days_since(&sig.last_accessed, now);
    let activation = score::activation_boost(
        score::activation(sig.access_count, days, cfg.rank.decay),
        clamp,
    );
    let affinity = file_affinity(
        conn,
        working_set,
        &sig.repo,
        &sig.path,
        clamp,
        affinity_cache,
    )?;
    let feedback = score::feedback_boost(score::beta_feedback(sig.used, sig.irrelevant), clamp);
    Ok(CodePriorParts {
        rank,
        activation,
        affinity,
        feedback,
        final_score: rank * activation * affinity * feedback,
    })
}

/// Median of the candidate pool's DISTINCT per-file `rank_score`s
/// (chunk rows share their file's projected score, so dedup is by
/// `(repo, path)`). Absolute PageRank scales with `1/file-count`, so a
/// fixed reference would make the boost depend on repo size;
/// median-relative mapping is repo-size invariant. Even-sized pools
/// take the mean of the middle two; an empty pool returns 0.0, which
/// [`rank_boost`] treats as "unranked repo → neutral".
pub fn median_file_rank<'a>(files: impl IntoIterator<Item = ((&'a str, &'a str), f64)>) -> f64 {
    let by_file: BTreeMap<(&str, &str), f64> = files.into_iter().collect();
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
    if ws.files().is_empty() {
        return Ok(1.0);
    }
    let fid = crate::graph::edges::file_node_id(repo, path);
    if let Some(boost) = cache.get(&fid) {
        return Ok(*boost);
    }
    let w_sum = co_change_weight(conn, &fid, ws.files())?;
    let boost = score::bounded_boost(1.0 + AFFINITY_SCALE * (1.0 + w_sum).ln(), clamp);
    cache.insert(fid, boost);
    Ok(boost)
}

/// Total `co_changed` weight between `fid` and the working-set file
/// ids, in either direction (the miner stores one canonical row per
/// undirected pair). Numbered placeholders are reused across both `IN`
/// lists so the parameter vector binds once; `prepare_cached` caches
/// one statement per working-set arity.
///
/// Arity-keyed caching tradeoff: the SQL string (and thus the cache
/// key) embeds the working-set length, so each distinct arity compiles
/// its own statement, and rusqlite's default cache capacity of 16 means
/// fluctuating arities can evict older entries (re-prepare churn, never
/// wrong results). Fine unless affinity shows up in a profile — revisit
/// with arity bucketing (pad the `IN` list to fixed sizes) if it does.
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
