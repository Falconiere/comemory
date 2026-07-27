use comemory::retrieval::fuse::{RankedHit, rrf, rrf_k, rrf_multi};
use proptest::prelude::*;

/// Build a ranked list from ids in rank order; `score` is unused by RRF.
fn hits(ids: &[&str]) -> Vec<RankedHit> {
    ids.iter()
        .map(|id| RankedHit {
            memory_id: (*id).into(),
            score: 0.0,
        })
        .collect()
}

/// Ids of a fused list, in output order.
fn ids(hits: &[RankedHit]) -> Vec<String> {
    hits.iter().map(|h| h.memory_id.clone()).collect()
}

#[test]
fn rrf_merges_two_ranked_lists() {
    let a = vec![
        RankedHit {
            memory_id: "a".into(),
            score: 0.0,
        },
        RankedHit {
            memory_id: "b".into(),
            score: 0.0,
        },
        RankedHit {
            memory_id: "c".into(),
            score: 0.0,
        },
    ];
    let b = vec![
        RankedHit {
            memory_id: "b".into(),
            score: 0.0,
        },
        RankedHit {
            memory_id: "c".into(),
            score: 0.0,
        },
        RankedHit {
            memory_id: "d".into(),
            score: 0.0,
        },
    ];
    let merged = rrf(&a, &b, 4);
    assert_eq!(merged.len(), 4);
    // `b` appears at rank 2 in `a` and rank 1 in `b`; `a` only at rank 1 in `a`.
    // `b` should win the merged top-1.
    assert_eq!(merged[0].memory_id, "b");
}

#[test]
fn rrf_empty_inputs_return_empty() {
    let merged = rrf(&[], &[], 10);
    assert!(merged.is_empty());
}

#[test]
fn rrf_truncates_to_top_k() {
    let a: Vec<RankedHit> = (0..10)
        .map(|i| RankedHit {
            memory_id: format!("a{i}"),
            score: 0.0,
        })
        .collect();
    let merged = rrf(&a, &[], 3);
    assert_eq!(merged.len(), 3);
}

#[test]
fn rrf_single_list_preserves_order() {
    let a = vec![
        RankedHit {
            memory_id: "x".into(),
            score: 0.0,
        },
        RankedHit {
            memory_id: "y".into(),
            score: 0.0,
        },
        RankedHit {
            memory_id: "z".into(),
            score: 0.0,
        },
    ];
    let merged = rrf(&a, &[], 3);
    assert_eq!(merged[0].memory_id, "x");
    assert_eq!(merged[1].memory_id, "y");
    assert_eq!(merged[2].memory_id, "z");
}

#[test]
fn rrf_multi_three_legs_accumulate_every_leg() {
    // k = 0 makes the contributions exact: 1/(rank+1).
    let l1 = hits(&["a", "b"]);
    let l2 = hits(&["c"]);
    let l3 = hits(&["b"]);
    // Two legs alone: a = 1.0, c = 1.0, b = 0.5 -> a, c, b.
    assert_eq!(ids(&rrf_multi(&[&l1, &l2], 3, 0.0)), ["a", "c", "b"]);
    // The third leg lifts b to 1.5 and flips it to the front.
    let merged = rrf_multi(&[&l1, &l2, &l3], 3, 0.0);
    assert_eq!(ids(&merged), ["b", "a", "c"]);
    assert!(
        (merged[0].score - 1.5).abs() < 1e-6,
        "b: {}",
        merged[0].score
    );
    assert!(
        (merged[1].score - 1.0).abs() < 1e-6,
        "a: {}",
        merged[1].score
    );
    assert!(
        (merged[2].score - 1.0).abs() < 1e-6,
        "c: {}",
        merged[2].score
    );
}

#[test]
fn rrf_multi_truncates_to_top_k() {
    let l1 = hits(&["a", "b"]);
    let l2 = hits(&["c", "d"]);
    let l3 = hits(&["e", "f"]);
    assert_eq!(rrf_multi(&[&l1, &l2, &l3], 2, 60.0).len(), 2);
}

#[test]
fn rrf_multi_empty_legs_slice_returns_empty() {
    assert!(rrf_multi(&[], 10, 60.0).is_empty());
}

#[test]
fn rrf_multi_ignores_empty_legs() {
    let a = hits(&["a", "b", "c"]);
    let b = hits(&["b", "d"]);
    let with_gaps = rrf_multi(&[&[], &a, &[], &b, &[]], 10, 60.0);
    let without = rrf_k(&a, &b, 10, 60.0);
    assert_eq!(ids(&with_gaps), ids(&without));
    for (x, y) in with_gaps.iter().zip(without.iter()) {
        assert!((x.score - y.score).abs() < 1e-6, "{x:?} vs {y:?}");
    }
    // All-empty legs are indistinguishable from no legs at all.
    assert!(rrf_multi(&[&[], &[]], 10, 60.0).is_empty());
}

#[test]
fn rrf_multi_single_leg_preserves_order() {
    let a = hits(&["x", "y", "z"]);
    assert_eq!(ids(&rrf_multi(&[&a], 3, 60.0)), ["x", "y", "z"]);
}

proptest! {
    /// AC-9: for arbitrary two-leg inputs `rrf_multi` reproduces `rrf_k`'s
    /// ordering exactly, with scores agreeing to 1e-6.
    #[test]
    fn rrf_multi_two_legs_matches_rrf_k(
        left in proptest::collection::vec("[a-e]{1,2}", 0..8),
        right in proptest::collection::vec("[a-e]{1,2}", 0..8),
        top_k in 0usize..12,
        k in 1.0f32..200.0,
    ) {
        let a: Vec<RankedHit> = left
            .iter()
            .map(|id| RankedHit { memory_id: id.clone(), score: 0.0 })
            .collect();
        let b: Vec<RankedHit> = right
            .iter()
            .map(|id| RankedHit { memory_id: id.clone(), score: 0.0 })
            .collect();
        let multi = rrf_multi(&[&a, &b], top_k, k);
        let pair = rrf_k(&a, &b, top_k, k);
        prop_assert_eq!(ids(&multi), ids(&pair));
        for (x, y) in multi.iter().zip(pair.iter()) {
            prop_assert!(
                (x.score - y.score).abs() <= 1e-6,
                "score drift for {}: {} vs {}",
                x.memory_id, x.score, y.score
            );
        }
    }
}
