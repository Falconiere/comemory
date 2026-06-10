//! Tests for [`comemory::retrieval::diversify::diversify`].

use comemory::retrieval::diversify::diversify;
use comemory::retrieval::rerank::{Reranked, ScoreParts};
use comemory::retrieval::router::Source;
use comemory::simhash::{hamming64, NEAR_DUP_HAMMING};

fn item(id: &str, score: f64, body: &str) -> Reranked {
    Reranked {
        memory_id: id.into(),
        source: Source::Lexical,
        parts: ScoreParts {
            rrf: score as f32,
            activation: 1.0,
            feedback: 1.0,
            quality: 1.0,
            supersede: 1.0,
            final_score: score,
        },
        superseded_by: None,
        body: body.into(),
        simhash: comemory::simhash::simhash64(comemory::simhash::tokens(body)),
    }
}

// ── Plan-specified tests ───────────────────────────────────────────────────

#[test]
fn near_duplicates_collapse_to_best_scored() {
    // Measured Hamming(a, b) = 8 — exactly at the NEAR_DUP_HAMMING boundary,
    // exercising the inclusive `<=` edge of the collapse. The identical-body
    // (Hamming 0) case is covered by `simhash_collapse_keeps_first_of_dup_group`.
    let a = item(
        "aaaa0001",
        0.9,
        "postgres connection pool exhausted under load spikes",
    );
    let b = item(
        "aaaa0002",
        0.5,
        "postgres connection pool exhausted under heavy load spikes",
    );
    let c = item(
        "aaaa0003",
        0.7,
        "rustfmt disagrees with clippy about line width",
    );
    let out = diversify(vec![a, b, c], 0.7, 10);
    let ids: Vec<&str> = out.iter().map(|r| r.memory_id.as_str()).collect();
    assert!(ids.contains(&"aaaa0001"), "best dup kept");
    assert!(!ids.contains(&"aaaa0002"), "worse dup collapsed");
    assert!(ids.contains(&"aaaa0003"));
}

#[test]
fn mmr_prefers_diverse_over_marginally_better() {
    // two near-identical topics + one distinct; k=2 must pick one of each.
    // Body `b` differs enough in token count to survive SimHash dedup while
    // still having high Jaccard overlap with `a` so MMR penalises it.
    let a = item("aaaa0001", 0.9, "sqlite fts5 tokenizer registration order");
    // Body b shares 5 of 7 unique tokens with a (Jaccard=0.714) so MMR
    // penalises it, and its SimHash is Hamming=13 away from a so SimHash
    // dedup does NOT remove it — the MMR stage decides.
    let b = item(
        "aaaa0002",
        0.89,
        "sqlite fts5 tokenizer registration order by sequence",
    );
    let c = item("aaaa0003", 0.6, "git hooks install path on windows runners");
    assert!(
        hamming64(a.simhash, b.simhash) > NEAR_DUP_HAMMING,
        "fixture must survive dedup so MMR decides"
    );
    let out = diversify(vec![a, b, c], 0.7, 2);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].memory_id, "aaaa0001");
    assert_eq!(out[1].memory_id, "aaaa0003");
}

#[test]
fn truncates_to_top_k() {
    let items: Vec<_> = (0..30)
        .map(|i| {
            item(
                &format!("aaaa{i:04}"),
                1.0 - i as f64 * 0.01,
                &format!("unique body {i} about topic {i}"),
            )
        })
        .collect();
    assert_eq!(diversify(items, 0.7, 12).len(), 12);
}

// ── Additional tests ───────────────────────────────────────────────────────

#[test]
fn empty_input_returns_empty_output() {
    let out = diversify(vec![], 0.7, 10);
    assert!(out.is_empty());
}

#[test]
fn lambda_one_preserves_relevance_order() {
    // lambda=1.0 means no diversity penalty, so order must match input
    // (which arrives sorted descending by final_score).
    let a = item("aaaa0001", 0.9, "completely unique topic one fish");
    let b = item("aaaa0002", 0.8, "completely unique topic two birds");
    let c = item("aaaa0003", 0.7, "completely unique topic three cats");
    let out = diversify(vec![a, b, c], 1.0, 3);
    let ids: Vec<&str> = out.iter().map(|r| r.memory_id.as_str()).collect();
    assert_eq!(ids, vec!["aaaa0001", "aaaa0002", "aaaa0003"]);
}

#[test]
fn equal_scores_break_toward_earlier_input() {
    // Two items with identical final_score and disjoint bodies (no Jaccard
    // penalty, no SimHash collapse): the MMR tie-break must pick the earlier
    // input, preserving the rerank stage's deterministic ordering.
    let a = item("aaaa0001", 0.8, "alpha bravo charlie delta");
    let b = item("aaaa0002", 0.8, "echo foxtrot golf hotel");
    let out = diversify(vec![a, b], 0.7, 1);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].memory_id, "aaaa0001", "earlier input wins the tie");
}

#[test]
fn simhash_collapse_keeps_first_of_dup_group() {
    // Input is sorted desc by score. The first item of a near-dup pair must
    // be kept; the second (lower score) must be dropped.
    let high = item(
        "aaaa0001",
        0.95,
        "postgres connection pool exhausted under load spikes",
    );
    let low = item(
        "aaaa0002",
        0.60,
        "postgres connection pool exhausted under load spikes",
    );
    // A completely different item to ensure the list is non-trivial.
    let other = item(
        "aaaa0003",
        0.50,
        "entirely different topic about memory allocation",
    );
    let out = diversify(vec![high, low, other], 1.0, 10);
    let ids: Vec<&str> = out.iter().map(|r| r.memory_id.as_str()).collect();
    // First of the dup group (aaaa0001, highest score) must survive.
    assert!(ids.contains(&"aaaa0001"), "first (highest) dup kept");
    // Second (lower score) must be collapsed.
    assert!(!ids.contains(&"aaaa0002"), "second (lower) dup removed");
    // Unrelated item must remain.
    assert!(ids.contains(&"aaaa0003"));
    // Order is still descending: aaaa0001 before aaaa0003 (pure lambda=1 pass).
    let pos0001 = ids.iter().position(|&x| x == "aaaa0001").unwrap();
    let pos0003 = ids.iter().position(|&x| x == "aaaa0003").unwrap();
    assert!(pos0001 < pos0003, "higher-scored item appears first");
}
