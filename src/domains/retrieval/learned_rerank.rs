//! The one learned ordering stage every search surface shares (#213).
//!
//! Built from `[rerank]`, invoked once per requested search, and never from
//! inside a retrieval leg — so a unified query cannot score twice. The stage
//! reorders only a fixed leading prefix of the deterministic ranking and
//! preserves the tail verbatim.
//!
//! Applied and refused share one code path: [`RerankOutcome::order_ids`] yields
//! the submitted order on a refusal, so [`apply`] restores the complete
//! original ranking as the identity permutation rather than through a second
//! branch that could drift.

use std::collections::HashMap;
use std::ffi::OsString;
use std::time::Duration;

use time::OffsetDateTime;

use crate::config::Config;
use crate::domains::learning::evaluation::candidate_facts::{self, FactsByHit};
use crate::domains::learning::evaluation::candidate_identity::CandidateDomain;
use crate::domains::retrieval::code_rerank::CodeReranked;
use crate::domains::retrieval::doc_route::DocHit;
use crate::domains::retrieval::learned_report::{LearnedOrdering, LearnedScore};
use crate::domains::retrieval::rerank::Reranked;
use crate::domains::retrieval::unified::fuse_domains::UnifiedHit;
use crate::prelude::*;
use crate::store::Connection;
use crate::utilities::rerank_outcome::RerankOutcome;
use crate::utilities::rerank_protocol::{RerankCandidate, RerankRequest};
use crate::utilities::rerank_runner::{RerankLimits, RerankRunner};

/// How a candidate's facts are addressed: its domain plus the id that domain's
/// own commands print — exactly how [`FactsByHit`] is keyed.
pub type CandidateKey = (CandidateDomain, String);

/// One configured learned ordering stage.
#[derive(Debug, Clone)]
pub struct LearnedStage {
    runner: RerankRunner,
    model: String,
    adapter: Option<String>,
    prefix: usize,
    text_bytes: usize,
}

impl LearnedStage {
    /// The stage this configuration implies, or `None` when reranking is off.
    ///
    /// `None` is the whole of the default-disabled guarantee: no
    /// [`RerankRunner`] is constructed, so no process can be launched and no
    /// candidate text is ever materialized.
    pub fn from_config(cfg: &Config) -> Option<Self> {
        if !cfg.rerank.enabled {
            return None;
        }
        let mut args = cfg.rerank.command.iter().map(OsString::from);
        let program = args.next()?;
        let limits = RerankLimits {
            // The stage never submits more than the configured prefix, so the
            // protocol ceiling is that prefix rather than #211's 256 default.
            max_candidates: cfg.rerank.prefix,
            max_candidate_text_bytes: cfg.rerank.max_candidate_text_bytes,
            ..RerankLimits::default()
        };
        Some(Self {
            runner: RerankRunner::new(program, args.collect())
                .with_timeout(Duration::from_millis(cfg.rerank.timeout_ms))
                .with_limits(limits),
            model: cfg.rerank.model.clone(),
            adapter: cfg.rerank.adapter().map(str::to_string),
            prefix: cfg.rerank.prefix,
            text_bytes: cfg.rerank.max_candidate_text_bytes,
        })
    }

    /// The per-candidate text bound this stage materializes at. A caller must
    /// collect facts at exactly this bound: a longer text would be refused by
    /// the runner, and a shorter one would score a different passage than the
    /// configuration declared.
    pub fn text_bytes(&self) -> usize {
        self.text_bytes
    }

    /// Build the scoring call for the leading prefix of `keys`, or `None` when
    /// there is nothing to score.
    pub fn plan(
        &self,
        query: &str,
        keys: &[CandidateKey],
        facts: &FactsByHit,
        now: OffsetDateTime,
    ) -> Option<LearnedCall> {
        let take = self.prefix.min(keys.len());
        if take == 0 {
            return None;
        }
        let candidates: Vec<RerankCandidate> = keys[..take]
            .iter()
            .enumerate()
            .map(|(rank, key)| {
                RerankCandidate::new(
                    wire_id(key),
                    u32::try_from(rank).unwrap_or(u32::MAX),
                    facts.get(key).map_or("", |f| f.text.text.as_str()),
                )
            })
            .collect();
        let refs = keys[..take]
            .iter()
            .map(|key| {
                facts
                    .get(key)
                    .map_or_else(|| wire_id(key), |f| f.identity.candidate_ref())
            })
            .collect();
        Some(LearnedCall {
            runner: self.runner.clone(),
            request: RerankRequest::new(
                self.model.clone(),
                self.adapter.clone(),
                query,
                candidates,
                now,
            ),
            refs,
        })
    }
}

/// A fully owned scoring call: the runner, the request, and nothing borrowed.
///
/// Exists so a caller holding a shared database lock can drop it, run
/// [`LearnedCall::score`], and take the lock again — see
/// `retrieval::staged`.
#[derive(Debug, Clone)]
pub struct LearnedCall {
    runner: RerankRunner,
    request: RerankRequest,
    refs: Vec<String>,
}

impl LearnedCall {
    /// Run the scorer child. Never fails: every operational failure is a
    /// [`RerankOutcome::Declined`] carrying the submitted order.
    pub fn score(&self) -> RerankOutcome {
        self.runner.rerank(&self.request)
    }

    /// The metadata [`apply`] needs once the call itself has been consumed.
    pub fn plan(&self) -> LearnedPlan {
        LearnedPlan {
            submitted: self.request.submitted_order(),
            refs: self.refs.clone(),
            model: self.request.model.clone(),
            adapter: self.request.adapter.clone(),
            request_id: self.request.request_id.clone(),
        }
    }
}

/// What was submitted, kept so an outcome can be applied after the call is gone.
#[derive(Debug, Clone)]
pub struct LearnedPlan {
    /// Wire ids in submitted (deterministic) order.
    pub submitted: Vec<String>,
    /// The candidate observation contract's reference string for each
    /// submitted candidate, in the same order. Reported, never matched on:
    /// unlike [`LearnedPlan::submitted`] it is not unique within a pool.
    pub refs: Vec<String>,
    /// The model identity that was asked for.
    pub model: String,
    /// The adapter identity that was asked for.
    pub adapter: Option<String>,
    /// The request id both sides agreed on.
    pub request_id: String,
}

/// Reorder the leading `plan.submitted.len()` entries of `ranked` into
/// `outcome`'s order and leave the tail exactly where the deterministic ranking
/// put it.
pub fn apply<T>(
    ranked: Vec<T>,
    plan: &LearnedPlan,
    outcome: &RerankOutcome,
) -> (Vec<T>, LearnedOrdering) {
    let pool = ranked.len();
    let prefix = plan.submitted.len();
    let report = report_of(plan, outcome, pool, prefix.min(pool));
    let positions: HashMap<&str, usize> = plan
        .submitted
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    let order = outcome.order_ids();
    // Validated BEFORE anything moves: a permutation is applied only when it is
    // a permutation of exactly the submitted prefix. Reordering first and
    // checking afterwards would have to put the moved hits back.
    if prefix > pool || !is_permutation(&order, &positions, prefix) {
        return (ranked, refused(plan, pool, prefix.min(pool)));
    }
    let mut slots: Vec<Option<T>> = ranked.into_iter().map(Some).collect();
    let mut out: Vec<T> = Vec::with_capacity(pool);
    for id in &order {
        if let Some(item) = positions.get(id).and_then(|i| slots.get_mut(*i)) {
            out.extend(item.take());
        }
    }
    out.extend(slots.into_iter().skip(prefix).flatten());
    (out, report)
}

/// Whether `order` names every submitted position exactly once.
fn is_permutation(order: &[&str], positions: &HashMap<&str, usize>, prefix: usize) -> bool {
    if order.len() != prefix {
        return false;
    }
    let mut seen = vec![false; prefix];
    order.iter().all(|id| {
        positions
            .get(id)
            .and_then(|i| seen.get_mut(*i))
            .is_some_and(|slot| !std::mem::replace(slot, true))
    })
}

/// The report for one outcome. Scores are reported only when the response was
/// applied: a refusal scored nothing.
fn report_of(
    plan: &LearnedPlan,
    outcome: &RerankOutcome,
    pool: usize,
    prefix: usize,
) -> LearnedOrdering {
    let (elapsed_ms, scores) = match outcome {
        RerankOutcome::Applied(applied) => (
            u64::try_from(applied.elapsed.as_millis()).unwrap_or(u64::MAX),
            applied
                .order
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let submitted = usize::try_from(c.rank).unwrap_or(usize::MAX);
                    LearnedScore {
                        candidate_id: c.id.clone(),
                        candidate_ref: plan
                            .refs
                            .get(submitted)
                            .cloned()
                            .unwrap_or_else(|| c.id.clone()),
                        rank: i + 1,
                        deterministic_rank: submitted.saturating_add(1),
                        score: c.score,
                    }
                })
                .collect(),
        ),
        RerankOutcome::Declined(_) => (0, Vec::new()),
    };
    LearnedOrdering {
        applied: outcome.is_applied(),
        model: plan.model.clone(),
        adapter: plan.adapter.clone(),
        request_id: plan.request_id.clone(),
        pool,
        prefix,
        elapsed_ms,
        fallback: outcome.failure().map(ToString::to_string),
        scores,
    }
}

/// The report for an order that was not a permutation of the submitted prefix.
///
/// Unreachable through the four surfaces — `rerank_validate` already proves an
/// applied order is an exact permutation — but [`apply`] is public and the
/// guard exists precisely for an order it cannot trust, so the report must say
/// the deterministic ranking stands rather than claim a reorder that did not
/// happen.
fn refused(plan: &LearnedPlan, pool: usize, prefix: usize) -> LearnedOrdering {
    LearnedOrdering {
        applied: false,
        model: plan.model.clone(),
        adapter: plan.adapter.clone(),
        request_id: plan.request_id.clone(),
        pool,
        prefix,
        elapsed_ms: 0,
        fallback: Some("reranked order is not a permutation of the submitted prefix".to_string()),
        scores: Vec::new(),
    }
}

/// The opaque id one candidate is offered under: its domain plus the id that
/// domain's own commands print.
///
/// Deliberately NOT the candidate observation contract's reference string,
/// which the protocol otherwise invites. That string is not injective over
/// `code_symbols`: its code form is `(repo, path, symbol, blob_oid)` while the
/// table's uniqueness key is `(repo, path, symbol, line_start)`, and the
/// extractor captures a bare identifier — so two same-named functions in one
/// file share a reference. #211 refuses a request with a repeated candidate id,
/// which would decline the whole stage for an ordinary code query. `(domain,
/// id)` is what keys the candidate pool in the first place and is therefore
/// unique within one request by construction. The reference string is still
/// reported, on [`LearnedScore::candidate_ref`].
fn wire_id(key: &CandidateKey) -> String {
    format!("{}:{}", key.0.as_str(), key.1)
}

/// The candidates one surface offers the stage: the leg rows its text is read
/// from, plus the fact keys in the final ranked order the stage must preserve.
///
/// The two are separate because they differ for `find`: its keys are the fused
/// order, which is not any single leg's.
#[derive(Debug, Clone, Copy, Default)]
pub struct Candidates<'a> {
    /// Memory rows whose bodies are the candidate text.
    pub memory: &'a [Reranked],
    /// Code rows whose `code_symbols` snippets are the candidate text.
    pub code: &'a [CodeReranked],
    /// Document rows whose winning-chunk passages are the candidate text.
    pub documents: &'a [DocHit],
    /// Fact keys in the ranked order the deterministic stage produced.
    pub keys: &'a [CandidateKey],
}

/// Materialize the candidate text and build the scoring call — the whole of a
/// surface's entry into the stage, so the four surfaces share one
/// implementation and cannot drift on how a candidate reaches the wire.
///
/// `None` for a disabled stage or an empty ranking, and in both cases nothing
/// was read from the store.
pub fn plan_call(
    stage: Option<&LearnedStage>,
    conn: &Connection,
    query: &str,
    candidates: Candidates<'_>,
) -> Result<Option<LearnedCall>> {
    let Some(stage) = stage else {
        return Ok(None);
    };
    if candidates.keys.is_empty() {
        return Ok(None);
    }
    let facts = candidate_facts::collect_parts(
        conn,
        candidates.memory,
        candidates.code,
        candidates.documents,
        stage.text_bytes(),
    )?;
    Ok(stage.plan(query, candidates.keys, &facts, OffsetDateTime::now_utc()))
}

/// Fact keys for a memory ranking, in its own order.
pub fn memory_keys(hits: &[Reranked]) -> Vec<CandidateKey> {
    hits.iter()
        .map(|h| (CandidateDomain::Memory, h.memory_id.clone()))
        .collect()
}

/// Fact keys for a code ranking, in its own order.
pub fn code_keys(hits: &[CodeReranked]) -> Vec<CandidateKey> {
    hits.iter()
        .map(|h| (CandidateDomain::Code, h.symbol_id.to_string()))
        .collect()
}

/// Fact keys for a fused unified ranking, in its own order.
pub fn unified_keys(hits: &[UnifiedHit]) -> Vec<CandidateKey> {
    hits.iter()
        .map(|h| (CandidateDomain::from_label(&h.domain), h.id.clone()))
        .collect()
}

#[cfg(test)]
#[path = "tests/learned_rerank.rs"]
mod tests;
