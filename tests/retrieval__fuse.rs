use comemory::retrieval::fuse::{RankedHit, rrf};

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
