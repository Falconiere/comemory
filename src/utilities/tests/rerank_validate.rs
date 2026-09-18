#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for response validation and tie-breaking.
//!
//! `validate` is pure, so these drive it with hand-built responses — including
//! the non-finite scores JSON has no literal for, which no subprocess could
//! produce and which the check must still refuse.

use comemory::utilities::rerank_outcome::RerankFailure;
use comemory::utilities::rerank_protocol::{
    RerankCandidate, RerankRequest, RerankResponse, RerankScore, ScoreDirection,
};
use comemory::utilities::rerank_validate::validate;

const REQUEST_ID: &str = "rr-20260918-1a2b3c4d";
const MODEL: &str = "bge-reranker-base";

fn request() -> RerankRequest {
    RerankRequest::with_request_id(
        REQUEST_ID,
        MODEL,
        None,
        "bounded subprocess",
        vec![
            RerankCandidate::new("memory:aaaaaaaa", 0, "first"),
            RerankCandidate::new("code:comemory:src/lib.rs:main", 1, "second"),
            RerankCandidate::new("document:notes.md#3", 2, "third"),
        ],
    )
}

fn response(scores: Vec<(&str, f64)>) -> RerankResponse {
    RerankResponse {
        protocol_version: 1,
        request_id: REQUEST_ID.to_string(),
        model: MODEL.to_string(),
        adapter: None,
        score_direction: ScoreDirection::HigherIsBetter,
        scores: scores
            .into_iter()
            .map(|(id, score)| RerankScore {
                id: id.to_string(),
                score,
            })
            .collect(),
    }
}

fn ids(request: &RerankRequest, response: &RerankResponse) -> Vec<String> {
    validate(request, response)
        .expect("valid")
        .into_iter()
        .map(|c| c.id)
        .collect()
}

#[test]
fn a_valid_response_reorders_by_score() {
    let request = request();
    let response = response(vec![
        ("memory:aaaaaaaa", 0.10),
        ("code:comemory:src/lib.rs:main", 0.90),
        ("document:notes.md#3", 0.50),
    ]);
    assert_eq!(
        ids(&request, &response),
        vec![
            "code:comemory:src/lib.rs:main",
            "document:notes.md#3",
            "memory:aaaaaaaa"
        ]
    );
}

#[test]
fn equal_scores_preserve_the_submitted_order() {
    let request = request();
    let mut response = response(vec![
        ("document:notes.md#3", 0.5),
        ("code:comemory:src/lib.rs:main", 0.5),
        ("memory:aaaaaaaa", 0.5),
    ]);
    // Even with the response listing them backwards, rank decides.
    assert_eq!(ids(&request, &response), request.submitted_order());
    response.score_direction = ScoreDirection::LowerIsBetter;
    assert_eq!(ids(&request, &response), request.submitted_order());
}

#[test]
fn lower_is_better_sorts_ascending_with_the_same_tie_break() {
    let request = request();
    let mut response = response(vec![
        ("memory:aaaaaaaa", 2.0),
        ("code:comemory:src/lib.rs:main", 1.0),
        ("document:notes.md#3", 1.0),
    ]);
    response.score_direction = ScoreDirection::LowerIsBetter;
    assert_eq!(
        ids(&request, &response),
        vec![
            "code:comemory:src/lib.rs:main",
            "document:notes.md#3",
            "memory:aaaaaaaa"
        ],
        "ascending by score, then ascending by rank — the rank order is never reversed"
    );
}

#[test]
fn a_version_mismatch_is_refused() {
    let mut response = response(vec![("memory:aaaaaaaa", 1.0)]);
    response.protocol_version = 2;
    assert!(matches!(
        validate(&request(), &response),
        Err(RerankFailure::VersionMismatch {
            expected: 1,
            actual: 2
        })
    ));
}

#[test]
fn a_request_id_mismatch_is_refused() {
    let mut response = response(vec![("memory:aaaaaaaa", 1.0)]);
    response.request_id = "rr-20260918-deadbeef".to_string();
    assert!(matches!(
        validate(&request(), &response),
        Err(RerankFailure::RequestIdMismatch { .. })
    ));
}

#[test]
fn a_model_mismatch_is_refused() {
    let mut response = response(vec![("memory:aaaaaaaa", 1.0)]);
    response.model = "some-other-model".to_string();
    match validate(&request(), &response) {
        Err(RerankFailure::ModelMismatch { expected, actual }) => {
            assert_eq!(expected, MODEL);
            assert_eq!(actual, "some-other-model");
        }
        other => panic!("expected ModelMismatch, got {other:?}"),
    }
}

#[test]
fn an_adapter_mismatch_is_refused_in_both_directions() {
    let base_request = request();
    let mut adapted = response(vec![("memory:aaaaaaaa", 1.0)]);
    adapted.adapter = Some("lora-v1".to_string());
    assert!(matches!(
        validate(&base_request, &adapted),
        Err(RerankFailure::AdapterMismatch { .. })
    ));

    let adapted_request = RerankRequest::with_request_id(
        REQUEST_ID,
        MODEL,
        Some("lora-v1".to_string()),
        "q",
        base_request.candidates.clone(),
    );
    let unadapted = response(vec![("memory:aaaaaaaa", 1.0)]);
    assert!(
        matches!(
            validate(&adapted_request, &unadapted),
            Err(RerankFailure::AdapterMismatch { .. })
        ),
        "an omitted adapter key must not pass for an adapted request"
    );
}

#[test]
fn a_missing_score_is_refused_and_names_every_gap() {
    let response = response(vec![("code:comemory:src/lib.rs:main", 1.0)]);
    match validate(&request(), &response) {
        Err(RerankFailure::MissingScores { ids }) => {
            assert_eq!(ids, vec!["memory:aaaaaaaa", "document:notes.md#3"]);
        }
        other => panic!("expected MissingScores, got {other:?}"),
    }
}

#[test]
fn a_duplicate_score_is_refused() {
    let response = response(vec![
        ("memory:aaaaaaaa", 1.0),
        ("code:comemory:src/lib.rs:main", 1.0),
        ("document:notes.md#3", 1.0),
        ("memory:aaaaaaaa", 2.0),
    ]);
    assert!(matches!(
        validate(&request(), &response),
        Err(RerankFailure::DuplicateScore { .. })
    ));
}

#[test]
fn an_unknown_score_is_refused() {
    let response = response(vec![
        ("memory:aaaaaaaa", 1.0),
        ("code:comemory:src/lib.rs:main", 1.0),
        ("document:notes.md#3", 1.0),
        ("memory:ffffffff", 1.0),
    ]);
    match validate(&request(), &response) {
        Err(RerankFailure::UnknownScore { id }) => assert_eq!(id, "memory:ffffffff"),
        other => panic!("expected UnknownScore, got {other:?}"),
    }
}

#[test]
fn non_finite_scores_are_refused() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let response = response(vec![
            ("memory:aaaaaaaa", 1.0),
            ("code:comemory:src/lib.rs:main", bad),
            ("document:notes.md#3", 1.0),
        ]);
        match validate(&request(), &response) {
            Err(RerankFailure::NonFiniteScore { id }) => {
                assert_eq!(id, "code:comemory:src/lib.rs:main");
            }
            other => panic!("expected NonFiniteScore for {bad}, got {other:?}"),
        }
    }
}

#[test]
fn an_overflowing_json_literal_cannot_smuggle_in_an_infinity() {
    // JSON has no NaN/Infinity literal, so the only way a non-finite value
    // could arrive over the wire is an overflowing exponent. Whichever way
    // serde_json resolves that, the outcome must not be an applied infinity.
    let raw = r#"{"protocol_version":1,"request_id":"rr-20260918-1a2b3c4d",
        "model":"bge-reranker-base","adapter":null,
        "score_direction":"higher_is_better",
        "scores":[{"id":"memory:aaaaaaaa","score":1e400},
                  {"id":"code:comemory:src/lib.rs:main","score":1.0},
                  {"id":"document:notes.md#3","score":1.0}]}"#;
    match serde_json::from_str::<RerankResponse>(raw) {
        Err(_) => {} // the parser refused it outright — also acceptable
        Ok(parsed) => {
            assert!(
                matches!(
                    validate(&request(), &parsed),
                    Err(RerankFailure::NonFiniteScore { .. })
                ),
                "an overflowing literal must not be applied"
            );
        }
    }
}

#[test]
fn a_repeated_id_is_a_duplicate_even_when_its_second_copy_is_non_finite() {
    // Structural defects outrank value defects, so the reported failure names
    // the real divergence (an id scored twice) rather than a symptom of it.
    let response = response(vec![
        ("memory:aaaaaaaa", 1.0),
        ("code:comemory:src/lib.rs:main", 1.0),
        ("document:notes.md#3", 1.0),
        ("memory:aaaaaaaa", f64::NAN),
    ]);
    match validate(&request(), &response) {
        Err(RerankFailure::DuplicateScore { id }) => assert_eq!(id, "memory:aaaaaaaa"),
        other => panic!("expected DuplicateScore, got {other:?}"),
    }
}

#[test]
fn negative_zero_ties_with_positive_zero_and_rank_decides() {
    // `-0.0` and `0.0` are numerically equal, but `total_cmp` orders the
    // negative first — which would silently steal the tie-break from `rank`.
    let request = request();
    let response = response(vec![
        ("document:notes.md#3", -0.0),
        ("code:comemory:src/lib.rs:main", 0.0),
        ("memory:aaaaaaaa", -0.0),
    ]);
    assert_eq!(
        ids(&request, &response),
        request.submitted_order(),
        "numerically equal scores must preserve the submitted order"
    );
}
