#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for the result vocabulary.
//!
//! The load-bearing property is that a declined outcome still answers "what
//! order should I use?" with the complete submitted order — that is what lets
//! retrieval restore its deterministic ranking from the failure alone.

use std::time::Duration;

use comemory::utilities::rerank_outcome::{
    RerankApplied, RerankDeclined, RerankFailure, RerankOutcome, RerankedCandidate,
};

fn submitted() -> Vec<String> {
    vec![
        "memory:aaaaaaaa".to_string(),
        "code:comemory:src/lib.rs:main".to_string(),
        "document:notes.md#3".to_string(),
    ]
}

fn declined(failure: RerankFailure) -> RerankOutcome {
    RerankOutcome::Declined(RerankDeclined {
        request_id: "rr-20260918-1a2b3c4d".to_string(),
        failure,
        original_order: submitted(),
        stderr_excerpt: "loading model...".to_string(),
    })
}

#[test]
fn a_declined_outcome_restores_the_complete_original_order() {
    let outcome = declined(RerankFailure::TimedOut { budget_ms: 20_000 });
    assert!(!outcome.is_applied());
    assert_eq!(
        outcome.order_ids(),
        vec![
            "memory:aaaaaaaa",
            "code:comemory:src/lib.rs:main",
            "document:notes.md#3"
        ]
    );
    assert!(matches!(
        outcome.failure(),
        Some(RerankFailure::TimedOut { budget_ms: 20_000 })
    ));
}

#[test]
fn an_applied_outcome_reports_the_reranked_order() {
    let outcome = RerankOutcome::Applied(RerankApplied {
        request_id: "rr-20260918-1a2b3c4d".to_string(),
        model: "bge-reranker-base".to_string(),
        adapter: None,
        order: vec![
            RerankedCandidate {
                id: "document:notes.md#3".to_string(),
                rank: 2,
                score: 0.91,
            },
            RerankedCandidate {
                id: "memory:aaaaaaaa".to_string(),
                rank: 0,
                score: 0.12,
            },
        ],
        stderr_excerpt: String::new(),
        elapsed: Duration::from_millis(42),
    });
    assert!(outcome.is_applied());
    assert!(outcome.failure().is_none());
    assert_eq!(
        outcome.order_ids(),
        vec!["document:notes.md#3", "memory:aaaaaaaa"]
    );
}

#[test]
fn every_failure_renders_one_useful_line() {
    // A caller logs the failure without matching every variant, so each one
    // must name the divergence rather than print a bare discriminant.
    let cases = [
        (RerankFailure::EmptyCandidates, vec!["no candidates"]),
        (
            RerankFailure::TooManyCandidates {
                count: 400,
                max: 256,
            },
            vec!["400", "256"],
        ),
        (RerankFailure::NonZeroExit { code: Some(3) }, vec!["3"]),
        (RerankFailure::NonZeroExit { code: None }, vec!["None"]),
        (
            RerankFailure::ModelMismatch {
                expected: "bge-reranker-base".to_string(),
                actual: "some-other-model".to_string(),
            },
            vec!["bge-reranker-base", "some-other-model"],
        ),
        (
            RerankFailure::AdapterMismatch {
                expected: Some("lora-v1".to_string()),
                actual: None,
            },
            vec!["lora-v1", "None"],
        ),
        (RerankFailure::MissingScores { ids: submitted() }, vec!["3"]),
        (
            RerankFailure::NonFiniteScore {
                id: "memory:aaaaaaaa".to_string(),
            },
            vec!["memory:aaaaaaaa"],
        ),
    ];
    for (failure, expected) in cases {
        let rendered = failure.to_string();
        assert!(!rendered.is_empty());
        assert!(!rendered.contains('\n'), "one line only: {rendered}");
        for needle in expected {
            assert!(
                rendered.contains(needle),
                "{rendered:?} should name {needle:?}"
            );
        }
    }
}
