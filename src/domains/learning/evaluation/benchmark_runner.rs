//! Capture one benchmark task's candidate pool through the real retrieval legs
//! and match its reviewed judgments against what came back.
//!
//! The run writes no telemetry at all: it calls `unified::run_legs`, which has
//! no tracking path, so `track: false` is structural rather than a flag. It
//! also issues the production-shaped page query once, purely to record whether
//! the benchmark's wider pool produced the same page a user would have seen.

use std::time::Instant;

use crate::config::Config;
use crate::domains::learning::evaluation::benchmark_metrics::MatchedJudgment;
use crate::domains::learning::evaluation::benchmark_observe::{effective_filters, observe};
use crate::domains::learning::evaluation::benchmark_set::{BenchmarkSet, BenchmarkTask};
use crate::domains::learning::evaluation::candidate_facts;
use crate::domains::learning::evaluation::candidate_observation::{
    CandidateObservation, OBSERVATION_VERSION, QueryObservation, RetrievalVersion,
};
use crate::domains::learning::evaluation::judgment::{MatchOutcome, TargetKey};
use crate::domains::retrieval::scope::{self, Filters};
use crate::domains::retrieval::unified::{
    self, DomainFilters, LegRows, UnifiedQuery, fuse_domains,
};
use crate::prelude::*;
use crate::store::Connection;
use crate::utilities::pagination::PageWindow;

/// What every task in one run shares.
#[derive(Debug)]
pub struct RunContext<'a> {
    /// The recall@k / nDCG@k cut.
    pub k: usize,
    /// The corpus, schema, binary and knob snapshot.
    pub version: &'a RetrievalVersion,
    /// RFC 3339 UTC instant the run started.
    pub reference_time: String,
}

/// One task's captured snapshot: the observation plus what the report needs
/// that is not part of the contract itself.
#[derive(Debug)]
pub struct TaskCapture {
    /// The task this capture belongs to.
    pub task_id: String,
    /// Every candidate retrieval produced, and how the query was posed.
    pub observation: QueryObservation,
    /// Whether the first `k` of the captured pool carry the same ids, in the
    /// same order, as the production-shaped page query returned. `false` is
    /// information — the production pool is narrower — not a failure.
    pub page_matches_production: bool,
    /// Judgments that survived staleness, with the positions they matched.
    pub matched: Vec<MatchedJudgment>,
    /// Targets of the judgments that matched nothing in the pool.
    pub unmatched_targets: Vec<String>,
    /// Judgments excluded because a pinned content version no longer holds.
    pub stale: usize,
    /// Candidates whose row vanished before their text could be read.
    pub text_unavailable: usize,
    /// Wall clock of the pool retrieval, excluding the parity query.
    pub retrieval_ms: u64,
}

/// Run one task and capture its pool.
///
/// `cfg` must already be the set's effective config, with the pinned `ranking`
/// applied; `shared` carries the run-wide version snapshot and reference time.
pub fn capture(
    cfg: &Config,
    conn: &Connection,
    set: &BenchmarkSet,
    task: &BenchmarkTask,
    shared: &RunContext<'_>,
) -> Result<TaskCapture> {
    let time_scope = scope::scope_from_flags(
        task.filters.since.as_deref(),
        task.filters.until.as_deref(),
        task.filters.as_of.as_deref(),
    )?;
    let query = UnifiedQuery {
        text: &task.query,
        vector: task.vector.as_deref(),
        filters: Filters {
            repo: task.filters.repo.as_deref(),
            kind: task.filters.kind.as_deref(),
            scope: &time_scope,
            domains: task.domain.mask(),
        },
        domain_filters: DomainFilters {
            lang: task.filters.lang.as_deref(),
            path_globs: &task.filters.path,
        },
    };

    let started = Instant::now();
    let legs = run_pool(cfg, conn, query)?;
    let facts = candidate_facts::collect(conn, &legs, set.defaults.max_text_bytes)?;
    let pool = unified::fuse_legs(cfg, conn, legs)?;
    let retrieval_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    let (candidates, text_unavailable) = observe(&pool, &facts, shared.k);
    let page_matches_production = page_matches(cfg, conn, query, shared.k, &pool)?;
    let judged = match_judgments(task, &candidates)?;
    Ok(TaskCapture {
        task_id: task.id.clone(),
        observation: QueryObservation {
            observation_version: OBSERVATION_VERSION,
            query_id: None,
            query: task.query.clone(),
            filters: effective_filters(task, set, &time_scope),
            retrieval: shared.version.clone(),
            reference_time: shared.reference_time.clone(),
            decay_frozen: shared.version.knobs.decay_frozen(),
            pool_size: cfg.retrieval.max_page_window,
            page_limit: shared.k,
            page_offset: 0,
            candidates,
        },
        page_matches_production,
        matched: judged.matched,
        unmatched_targets: judged.unmatched_targets,
        stale: judged.stale,
        text_unavailable,
        retrieval_ms,
    })
}

/// Run every in-scope leg over the whole ranked window. `limit: 0` is what
/// `pipeline::pool_size` resolves to `max_page_window`, so the captured pool is
/// the deepest list retrieval will build for this query.
fn run_pool(cfg: &Config, conn: &Connection, query: UnifiedQuery<'_>) -> Result<LegRows> {
    let window = PageWindow {
        offset: 0,
        limit: 0,
    };
    unified::run_legs(cfg, conn, query, window)
}

/// Whether the production-shaped page query returns the captured pool's own
/// prefix. Issued separately and excluded from every latency figure.
fn page_matches(
    cfg: &Config,
    conn: &Connection,
    query: UnifiedQuery<'_>,
    k: usize,
    pool: &[fuse_domains::UnifiedHit],
) -> Result<bool> {
    let window = PageWindow {
        offset: 0,
        limit: k,
    };
    let production = unified::find(cfg, conn, query, window)?;
    Ok(production
        .hits
        .iter()
        .map(|h| (h.domain.as_str(), h.id.as_str()))
        .eq(pool
            .iter()
            .take(k)
            .map(|h| (h.domain.as_str(), h.id.as_str()))))
}

/// The outcome of matching one task's judgments against its pool.
struct Judged {
    matched: Vec<MatchedJudgment>,
    unmatched_targets: Vec<String>,
    stale: usize,
}

/// Match every judgment against the pool. A judgment whose identity agrees but
/// whose pinned version does not is stale: dropped from scoring entirely and
/// counted, never treated as a hit.
fn match_judgments(task: &BenchmarkTask, candidates: &[CandidateObservation]) -> Result<Judged> {
    let keys = task.target_keys()?;
    let mut judged = Judged {
        matched: Vec::with_capacity(keys.len()),
        unmatched_targets: Vec::new(),
        stale: 0,
    };
    for (key, judgment) in keys.iter().zip(task.judgments.iter()) {
        let mut positions = Vec::new();
        let mut saw_stale = false;
        for candidate in candidates {
            match key.matches(&candidate.identity) {
                MatchOutcome::Yes => positions.push(candidate.pool_position),
                MatchOutcome::Stale => saw_stale = true,
                MatchOutcome::No => {}
            }
        }
        if positions.is_empty() && saw_stale {
            judged.stale += 1;
            continue;
        }
        if positions.is_empty() {
            judged.unmatched_targets.push(describe(key));
        }
        judged.matched.push(MatchedJudgment {
            relevance: judgment.relevance,
            domain: key.domain(),
            pool_positions: positions,
        });
    }
    Ok(judged)
}

/// Render a target's own keys for the unmatched report. An unmatched judgment
/// has no observed content version, so it has no candidate reference.
fn describe(key: &TargetKey) -> String {
    match key {
        TargetKey::Memory { id, .. } => format!("memory:{id}"),
        TargetKey::Code {
            repo, path, symbol, ..
        } => format!("code:{repo}:{path}:{symbol}"),
        TargetKey::Document { path, .. } => format!("document:{path}"),
    }
}

#[cfg(test)]
#[path = "tests/benchmark_runner.rs"]
mod tests;
