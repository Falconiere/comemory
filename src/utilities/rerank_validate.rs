//! Validate a reranker response against the request that produced it.
//!
//! Pure, with no process I/O, so every rejection rule is exercisable from a
//! hand-built response. A response is applied only after all of it passes.

use std::collections::{HashMap, HashSet};

use crate::utilities::rerank_outcome::{RerankFailure, RerankedCandidate};
use crate::utilities::rerank_protocol::{
    RerankRequest, RerankResponse, RerankScore, ScoreDirection,
};

/// Check `response` against `request` and return the reranked order.
///
/// Checks run most-specific-first — version, request id, model, adapter, then
/// per score, then completeness — so the reported failure names the real
/// divergence rather than a downstream symptom of it.
pub fn validate(
    request: &RerankRequest,
    response: &RerankResponse,
) -> Result<Vec<RerankedCandidate>, RerankFailure> {
    check_identity(request, response)?;
    let scores = collect_scores(request, response)?;
    let mut ranked = rank_all(request, &scores)?;
    sort_ranked(&mut ranked, response.score_direction);
    Ok(ranked)
}

/// Refuse a response that did not come from the request we sent, or from the
/// model and adapter we expected.
fn check_identity(request: &RerankRequest, response: &RerankResponse) -> Result<(), RerankFailure> {
    if response.protocol_version != request.protocol_version {
        return Err(RerankFailure::VersionMismatch {
            expected: request.protocol_version,
            actual: response.protocol_version,
        });
    }
    if response.request_id != request.request_id {
        return Err(RerankFailure::RequestIdMismatch {
            expected: request.request_id.clone(),
            actual: response.request_id.clone(),
        });
    }
    if response.model != request.model {
        return Err(RerankFailure::ModelMismatch {
            expected: request.model.clone(),
            actual: response.model.clone(),
        });
    }
    if response.adapter != request.adapter {
        return Err(RerankFailure::AdapterMismatch {
            expected: request.adapter.clone(),
            actual: response.adapter.clone(),
        });
    }
    Ok(())
}

/// Index the scores by candidate id, refusing an unknown id, a repeated id or
/// a non-finite value.
fn collect_scores<'a>(
    request: &RerankRequest,
    response: &'a RerankResponse,
) -> Result<HashMap<&'a str, f64>, RerankFailure> {
    let offered: HashSet<&str> = request.candidates.iter().map(|c| c.id.as_str()).collect();
    let mut scores: HashMap<&str, f64> = HashMap::with_capacity(response.scores.len());
    for RerankScore { id, score } in &response.scores {
        if !offered.contains(id.as_str()) {
            return Err(RerankFailure::UnknownScore { id: id.clone() });
        }
        // Structural defects before value defects: a repeated id is reported as
        // a duplicate even when its second copy is also non-finite.
        if scores.insert(id.as_str(), *score).is_some() {
            return Err(RerankFailure::DuplicateScore { id: id.clone() });
        }
        if !score.is_finite() {
            return Err(RerankFailure::NonFiniteScore { id: id.clone() });
        }
    }
    Ok(scores)
}

/// Pair every submitted candidate with its score, refusing any that is absent.
fn rank_all(
    request: &RerankRequest,
    scores: &HashMap<&str, f64>,
) -> Result<Vec<RerankedCandidate>, RerankFailure> {
    let missing: Vec<String> = request
        .candidates
        .iter()
        .filter(|c| !scores.contains_key(c.id.as_str()))
        .map(|c| c.id.clone())
        .collect();
    if !missing.is_empty() {
        return Err(RerankFailure::MissingScores { ids: missing });
    }
    Ok(request
        .candidates
        .iter()
        .filter_map(|c| {
            scores.get(c.id.as_str()).map(|score| RerankedCandidate {
                id: c.id.clone(),
                rank: c.rank,
                score: *score,
            })
        })
        .collect())
}

/// Order by score in the declared direction, breaking ties by the submitted
/// rank ascending.
///
/// Every score is already proven finite, so `total_cmp` is a total order and no
/// comparator can panic. The rank comparison is never reversed: numerically
/// equal scores always preserve the caller's deterministic order — which is why
/// the sign of zero is normalized first. `total_cmp` orders `-0.0` before
/// `0.0` even though the two are numerically equal, and that would silently
/// steal the tie-break from `rank`.
fn sort_ranked(ranked: &mut [RerankedCandidate], direction: ScoreDirection) {
    ranked.sort_by(|a, b| {
        let (left, right) = (unsign_zero(a.score), unsign_zero(b.score));
        let by_score = match direction {
            ScoreDirection::HigherIsBetter => right.total_cmp(&left),
            ScoreDirection::LowerIsBetter => left.total_cmp(&right),
        };
        by_score.then(a.rank.cmp(&b.rank))
    });
}

/// Map `-0.0` onto `0.0` and leave every other finite value alone, so that
/// numerically equal scores compare equal under `total_cmp`.
fn unsign_zero(score: f64) -> f64 {
    if score == 0.0 { 0.0 } else { score }
}

#[cfg(test)]
#[path = "tests/rerank_validate.rs"]
mod tests;
