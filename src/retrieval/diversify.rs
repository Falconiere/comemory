//! Third retrieval stage: collapse SimHash near-duplicates, then apply
//! MMR (maximal marginal relevance) with token-set Jaccard similarity,
//! and cut to top-k. Embedding-free by design.

use std::collections::HashSet;

use crate::retrieval::rerank::Reranked;
use crate::simhash::{hamming64, NEAR_DUP_HAMMING};

/// Collapse near-duplicates, then greedily select up to `top_k` items
/// maximizing `lambda·score − (1−lambda)·max_jaccard_to_selected`.
/// Input must already be sorted by final score descending (rerank output).
pub fn diversify(items: Vec<Reranked>, lambda: f64, top_k: usize) -> Vec<Reranked> {
    let deduped = collapse_near_dups(items);
    mmr(deduped, lambda, top_k)
}

/// Remove near-duplicate entries, keeping the first (highest-scored)
/// representative of each SimHash cluster. Input is expected to arrive
/// sorted descending by final score so the best variant is always
/// encountered before its weaker duplicates.
fn collapse_near_dups(items: Vec<Reranked>) -> Vec<Reranked> {
    let mut kept: Vec<Reranked> = Vec::with_capacity(items.len());
    for item in items {
        // items arrive best-first, so the first of a dup group wins
        let dup = kept
            .iter()
            .any(|k| hamming64(k.simhash, item.simhash) <= NEAR_DUP_HAMMING);
        if !dup {
            kept.push(item);
        }
    }
    kept
}

/// Build a token set from a body string for Jaccard computation.
fn token_set(body: &str) -> HashSet<String> {
    crate::simhash::tokens(body).into_iter().collect()
}

/// Jaccard similarity between two token sets.
fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    inter / union
}

/// Greedy MMR selection: at each step pick the candidate with the highest
/// `lambda * relevance - (1 - lambda) * max_jaccard_to_selected`. Equal MMR
/// scores break toward the earlier (more relevant) candidate via an index
/// tie-break.
///
/// Relevance is min-max normalized within the candidate pool before
/// selection (see [`normalized_relevance`]); the returned items keep their
/// original `parts.final_score` untouched — normalization is selection-only
/// and never leaks into the output contract.
fn mmr(items: Vec<Reranked>, lambda: f64, top_k: usize) -> Vec<Reranked> {
    let relevance = normalized_relevance(&items);
    let sets: Vec<HashSet<String>> = items.iter().map(|i| token_set(&i.body)).collect();
    let mut remaining: Vec<usize> = (0..items.len()).collect();
    let mut picked_idx: Vec<usize> = Vec::with_capacity(top_k.min(items.len()));

    while picked_idx.len() < top_k && !remaining.is_empty() {
        let Some(pos) = remaining
            .iter()
            .enumerate()
            .max_by(|(ia, &a), (ib, &b)| {
                let sa = mmr_score(&relevance, &sets, &picked_idx, a, lambda);
                let sb = mmr_score(&relevance, &sets, &picked_idx, b, lambda);
                sa.total_cmp(&sb).then_with(|| ib.cmp(ia))
            })
            .map(|(pos, _)| pos)
        else {
            break;
        };

        picked_idx.push(remaining[pos]);
        remaining.remove(pos);
    }

    let mut slots: Vec<Option<Reranked>> = items.into_iter().map(Some).collect();
    picked_idx
        .into_iter()
        .filter_map(|i| slots[i].take())
        .collect()
}

/// Min-max normalize each item's `final_score` into `[0, 1]` for MMR
/// selection only.
///
/// Rationale: the relevance term mixes wildly different scales depending
/// on the routing branch — RRF fusion yields scores around `1/k ≈ 0.016`
/// while the pure-lexical branch carries `-bm25` values anywhere from
/// `1e-6` to `10+`. Against the Jaccard diversity term (always `[0, 1]`),
/// unnormalized relevance would either vanish (RRF) or dominate (lexical),
/// making `lambda` mean something different per branch. Normalizing within
/// the candidate pool makes the trade-off branch-stable and, because
/// min-max is affine-invariant, insensitive to the absolute score scale.
///
/// When all scores are equal (or the range is degenerate/non-finite) every
/// item gets `1.0` so selection falls back to diversity plus the index
/// tie-break.
fn normalized_relevance(items: &[Reranked]) -> Vec<f64> {
    let min = items
        .iter()
        .map(|i| i.parts.final_score)
        .fold(f64::INFINITY, f64::min);
    let max = items
        .iter()
        .map(|i| i.parts.final_score)
        .fold(f64::NEG_INFINITY, f64::max);
    let range = max - min;
    if !range.is_finite() || range <= 0.0 {
        return vec![1.0; items.len()];
    }
    items
        .iter()
        .map(|i| (i.parts.final_score - min) / range)
        .collect()
}

/// MMR objective for one candidate given the set already selected.
/// `relevance` is the pool-normalized score from [`normalized_relevance`],
/// not the raw `final_score`.
fn mmr_score(
    relevance: &[f64],
    sets: &[HashSet<String>],
    picked: &[usize],
    candidate: usize,
    lambda: f64,
) -> f64 {
    let max_sim = picked
        .iter()
        .map(|&p| jaccard(&sets[candidate], &sets[p]))
        .fold(0.0f64, f64::max);
    lambda * relevance[candidate] - (1.0 - lambda) * max_sim
}
