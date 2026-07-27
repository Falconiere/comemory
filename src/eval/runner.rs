//! Drive the real retrieval pipeline over a golden set and score it.

use rusqlite::Connection;
use serde::Serialize;

use crate::config::Config;
use crate::eval::golden::GoldenPair;
use crate::eval::metrics;
use crate::prelude::*;
use crate::retrieval::pipeline::{self, SearchOptions};
use crate::retrieval::scope::Filters;

/// Per-query eval outcome, serialized into the `--json` report.
#[derive(Debug, Serialize)]
pub struct QueryResult {
    /// Golden query text.
    pub query: String,
    /// Golden relevant ids.
    pub relevant: Vec<String>,
    /// Ids the pipeline returned, in rank order.
    pub returned: Vec<String>,
    /// One-based rank of the first relevant hit, if any.
    pub rank_of_first_hit: Option<usize>,
    /// recall@k for this query.
    pub recall: f64,
}

/// Aggregate eval report. `recall_at_k` and `mrr` are means over queries.
#[derive(Debug, Serialize)]
pub struct EvalReport {
    /// The k used for recall@k.
    pub k: usize,
    /// Mean recall@k over all golden queries.
    pub recall_at_k: f64,
    /// Mean reciprocal rank over all golden queries (miss = 0).
    pub mrr: f64,
    /// Number of golden queries evaluated.
    pub queries: usize,
    /// Per-query breakdown, worst-first by recall then query text.
    pub results: Vec<QueryResult>,
}

/// Run every golden query through the real pipeline (`track: false` —
/// measurement must not feed the signals it measures) and aggregate
/// recall@k + MRR. Each pair's originating `repo`/`kind` filters are
/// replayed verbatim. Lexical path only: BYO vectors cannot be replayed
/// offline.
pub fn run_eval(
    cfg: &Config,
    conn: &Connection,
    pairs: &[GoldenPair],
    k: usize,
) -> Result<EvalReport> {
    let mut results = Vec::with_capacity(pairs.len());
    let mut recall_sum = 0.0;
    let mut rr_sum = 0.0;
    for pair in pairs {
        let scored = score_pair(cfg, conn, pair, k)?;
        recall_sum += scored.recall;
        rr_sum += scored.rank_of_first_hit.map_or(0.0, |r| 1.0 / r as f64);
        results.push(scored);
    }
    let n = pairs.len().max(1) as f64;
    results.sort_by(|a, b| {
        a.recall
            .total_cmp(&b.recall)
            .then_with(|| a.query.cmp(&b.query))
    });
    Ok(EvalReport {
        k,
        recall_at_k: recall_sum / n,
        mrr: rr_sum / n,
        queries: pairs.len(),
        results,
    })
}

/// Replay one golden pair through the pipeline (unscoped in time — eval
/// measures the present-day corpus) and score it against its relevant ids.
fn score_pair(cfg: &Config, conn: &Connection, pair: &GoldenPair, k: usize) -> Result<QueryResult> {
    let run = pipeline::search(
        cfg,
        conn,
        &pair.query,
        None,
        Filters {
            repo: pair.repo.as_deref(),
            kind: pair.kind.as_deref(),
            ..Filters::none()
        },
        SearchOptions {
            track: false,
            source: crate::stats::source::SEARCH,
            // Eval scores the unpaginated first page (the historical
            // `top_k` cut), so metrics stay comparable across runs.
            window: pipeline::PageWindow::top_k(cfg),
        },
    )?;
    let returned: Vec<String> = run.hits.iter().map(|h| h.memory_id.clone()).collect();
    let recall = metrics::recall_at_k(&pair.relevant, &returned, k);
    Ok(QueryResult {
        rank_of_first_hit: metrics::first_hit_rank(&pair.relevant, &returned),
        query: pair.query.clone(),
        relevant: pair.relevant.clone(),
        returned,
        recall,
    })
}
