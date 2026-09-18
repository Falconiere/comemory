#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for the wire types.
//!
//! The JSON these produce is the contract #212, #213 and #214 build against,
//! so the assertions are on the serialized keys and values themselves, not on
//! a round trip that would pass for any consistent shape.

use comemory::utilities::rerank_protocol::{
    RERANK_PROTOCOL_VERSION, RerankCandidate, RerankRequest, RerankResponse, ScoreDirection,
    is_valid_request_id,
};
use serde_json::{Value, json};

fn candidates() -> Vec<RerankCandidate> {
    vec![
        RerankCandidate::new("memory:679929eb", 0, "domain migration notes"),
        RerankCandidate::new(
            "code:comemory:src/utilities/embed.rs:embed_query",
            1,
            "fn embed_query(cmd: &str, query: &str)",
        ),
    ]
}

#[test]
fn a_minted_request_carries_the_documented_shape() {
    let now = time::macros::datetime!(2026-09-18 12:34:56.789 UTC);
    let request = RerankRequest::new(
        "bge-reranker-base",
        None,
        "bounded subprocess",
        candidates(),
        now,
    );

    assert_eq!(request.protocol_version, RERANK_PROTOCOL_VERSION);
    assert_eq!(request.protocol_version, 1);
    assert!(
        is_valid_request_id(&request.request_id),
        "{}",
        request.request_id
    );
    assert!(request.request_id.starts_with("rr-20260918-"));
    assert_eq!(request.candidates[0].rank, 0);
    assert_eq!(request.candidates[1].rank, 1);
    assert_eq!(
        request.submitted_order(),
        vec![
            "memory:679929eb".to_string(),
            "code:comemory:src/utilities/embed.rs:embed_query".to_string()
        ]
    );
}

#[test]
fn the_request_serializes_to_exactly_the_documented_keys() {
    let request = RerankRequest::with_request_id(
        "rr-20260918-1a2b3c4d",
        "bge-reranker-base",
        None,
        "bounded subprocess",
        candidates(),
    );
    let value: Value = serde_json::to_value(&request).unwrap();

    let object = value.as_object().expect("a JSON object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "adapter",
            "candidates",
            "model",
            "protocol_version",
            "query",
            "request_id"
        ]
    );
    assert_eq!(object["protocol_version"], json!(1));
    assert_eq!(object["request_id"], json!("rr-20260918-1a2b3c4d"));
    assert_eq!(object["model"], json!("bge-reranker-base"));
    assert_eq!(
        object["adapter"],
        Value::Null,
        "a base-model run sends null"
    );
    assert_eq!(object["query"], json!("bounded subprocess"));

    let first = &object["candidates"][0];
    let mut candidate_keys: Vec<&str> = first
        .as_object()
        .expect("a candidate object")
        .keys()
        .map(String::as_str)
        .collect();
    candidate_keys.sort_unstable();
    assert_eq!(candidate_keys, vec!["id", "rank", "text"]);
    assert_eq!(first["id"], json!("memory:679929eb"));
    assert_eq!(first["rank"], json!(0));
}

#[test]
fn an_adapter_serializes_as_its_own_string() {
    let request = RerankRequest::with_request_id(
        "rr-20260918-1a2b3c4d",
        "bge-reranker-base",
        Some("comemory-lora-v1".to_string()),
        "q",
        candidates(),
    );
    let value: Value = serde_json::to_value(&request).unwrap();
    assert_eq!(value["adapter"], json!("comemory-lora-v1"));
}

#[test]
fn a_documented_response_parses() {
    let raw = r#"{"protocol_version":1,"request_id":"rr-20260918-1a2b3c4d",
        "model":"bge-reranker-base","adapter":null,
        "score_direction":"higher_is_better",
        "scores":[{"id":"memory:679929eb","score":0.13},
                  {"id":"code:comemory:src/utilities/embed.rs:embed_query","score":0.87}]}"#;
    let response: RerankResponse = serde_json::from_str(raw).unwrap();
    assert_eq!(response.protocol_version, 1);
    assert_eq!(response.score_direction, ScoreDirection::HigherIsBetter);
    assert_eq!(response.scores.len(), 2);
    assert_eq!(response.scores[1].score, 0.87);
}

#[test]
fn an_extra_response_field_is_rejected() {
    // deny_unknown_fields is what makes the version number meaningful: an
    // additive field has to be a version bump, not a silent extension.
    let raw = r#"{"protocol_version":1,"request_id":"rr-20260918-1a2b3c4d",
        "model":"m","adapter":null,"score_direction":"lower_is_better",
        "scores":[],"calibration":"sigmoid"}"#;
    let err = serde_json::from_str::<RerankResponse>(raw).unwrap_err();
    assert!(err.to_string().contains("calibration"), "{err}");
}

#[test]
fn an_unknown_score_direction_is_rejected() {
    let raw = r#"{"protocol_version":1,"request_id":"rr-20260918-1a2b3c4d",
        "model":"m","adapter":null,"score_direction":"closest_to_zero","scores":[]}"#;
    assert!(serde_json::from_str::<RerankResponse>(raw).is_err());
}

#[test]
fn a_non_object_response_is_rejected_rather_than_panicking() {
    for raw in ["", "[]", "null", "not json", "{\"protocol_version\":1}"] {
        assert!(
            serde_json::from_str::<RerankResponse>(raw).is_err(),
            "must reject {raw:?}"
        );
    }
}
