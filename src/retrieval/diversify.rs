//! Third retrieval stage: collapse SimHash near-duplicates, then apply
//! MMR (maximal marginal relevance) with token-set Jaccard similarity,
//! and cut to top-k. Embedding-free by design.

use std::collections::HashSet;

use crate::retrieval::rerank::Reranked;
use crate::simhash::hamming64;

/// Collapse near-duplicates within `near_dup_hamming` SimHash bits
/// (callers pass `cfg.rank.near_dup_hamming`, which defaults to
/// [`crate::simhash::NEAR_DUP_HAMMING`]), then greedily select up to
/// `top_k` items maximizing
/// `lambda·score − (1−lambda)·max_jaccard_to_selected`.
/// Input must already be sorted by final score descending (rerank output).
pub fn diversify(
    items: Vec<Reranked>,
    near_dup_hamming: u32,
    lambda: f64,
    top_k: usize,
) -> Vec<Reranked> {
    let deduped = collapse_near_dups(items, near_dup_hamming);
    mmr(deduped, lambda, top_k)
}

/// Remove entries within `near_dup_hamming` SimHash bits of a kept one,
/// keeping the first (highest-scored) representative of each cluster.
/// Input is expected to arrive sorted descending by final score so the
/// best variant is always encountered before its weaker duplicates.
fn collapse_near_dups(items: Vec<Reranked>, near_dup_hamming: u32) -> Vec<Reranked> {
    let mut kept: Vec<Reranked> = Vec::with_capacity(items.len());
    for item in items {
        // items arrive best-first, so the first of a dup group wins
        let dup = kept
            .iter()
            .any(|k| hamming64(k.simhash, item.simhash) <= near_dup_hamming);
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
/// selection (see [`crate::retrieval::score::min_max_normalize`] for the
/// rationale); the returned items keep their original `parts.final_score`
/// untouched — normalization is selection-only and never leaks into the
/// output contract. This re-normalizes scores the rerank stage already
/// normalized: intentional, because `final_score` after priors is no
/// longer in `[0, 1]`.
///
/// `max_sim` (each candidate's max Jaccard to the picked set) is kept
/// incrementally: after every pick, each remaining candidate is compared
/// against only the newest pick — one Jaccard per remaining candidate per
/// round (O(n·k) total) instead of recomputing the whole picked set every
/// round (O(n·k²)). Selection semantics are bitwise-identical to the
/// recomputing form, including tie-breaks.
fn mmr(items: Vec<Reranked>, lambda: f64, top_k: usize) -> Vec<Reranked> {
    let relevance = crate::retrieval::score::min_max_normalize(
        &items
            .iter()
            .map(|i| i.parts.final_score)
            .collect::<Vec<_>>(),
    );
    let sets: Vec<HashSet<String>> = items.iter().map(|i| token_set(&i.body)).collect();
    let mut remaining: Vec<usize> = (0..items.len()).collect();
    let mut max_sim: Vec<f64> = vec![0.0; items.len()];
    let mut picked_idx: Vec<usize> = Vec::with_capacity(top_k.min(items.len()));

    while picked_idx.len() < top_k && !remaining.is_empty() {
        let Some(pos) = remaining
            .iter()
            .enumerate()
            .max_by(|(ia, &a), (ib, &b)| {
                let sa = lambda * relevance[a] - (1.0 - lambda) * max_sim[a];
                let sb = lambda * relevance[b] - (1.0 - lambda) * max_sim[b];
                sa.total_cmp(&sb).then_with(|| ib.cmp(ia))
            })
            .map(|(pos, _)| pos)
        else {
            break;
        };

        let newest = remaining[pos];
        picked_idx.push(newest);
        remaining.remove(pos);
        for &candidate in &remaining {
            max_sim[candidate] = max_sim[candidate].max(jaccard(&sets[candidate], &sets[newest]));
        }
    }

    let mut slots: Vec<Option<Reranked>> = items.into_iter().map(Some).collect();
    picked_idx
        .into_iter()
        .filter_map(|i| slots[i].take())
        .collect()
}
