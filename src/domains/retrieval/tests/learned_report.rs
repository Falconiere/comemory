#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::retrieval::learned_report`] — the JSON shape every
//! retrieval envelope carries under its optional `learned` key, and the one
//! TTY summary line the four renderers share.

use comemory::retrieval::learned_report::{LearnedOrdering, LearnedScore};

fn applied() -> LearnedOrdering {
    LearnedOrdering {
        applied: true,
        model: "lexical-overlap@1".into(),
        adapter: None,
        request_id: "rr-20260918-1a2b3c4d".into(),
        pool: 200,
        prefix: 2,
        elapsed_ms: 83,
        fallback: None,
        scores: vec![
            LearnedScore {
                candidate_id: "memory:aaaa0001".into(),
                candidate_ref: "memory:aaaa0001:abcd".into(),
                rank: 1,
                deterministic_rank: 2,
                score: 0.5,
            },
            LearnedScore {
                candidate_id: "memory:bbbb0002".into(),
                candidate_ref: "memory:bbbb0002:efgh".into(),
                rank: 2,
                deterministic_rank: 1,
                score: 0.25,
            },
        ],
    }
}

fn declined() -> LearnedOrdering {
    LearnedOrdering {
        applied: false,
        model: "cross-encoder/ms-marco-MiniLM-L6-v2@233902d2".into(),
        adapter: Some("lora-v1".into()),
        request_id: "rr-20260918-1a2b3c4d".into(),
        pool: 200,
        prefix: 50,
        elapsed_ms: 0,
        fallback: Some("reranker timed out after 300ms".into()),
        scores: Vec::new(),
    }
}

#[test]
fn an_applied_report_carries_every_distinct_field() {
    let v = serde_json::to_value(applied()).expect("serialize");
    assert_eq!(v["applied"], serde_json::json!(true));
    assert_eq!(v["model"], serde_json::json!("lexical-overlap@1"));
    assert_eq!(v["adapter"], serde_json::Value::Null);
    assert_eq!(v["pool"], serde_json::json!(200));
    assert_eq!(v["prefix"], serde_json::json!(2));
    assert_eq!(v["elapsed_ms"], serde_json::json!(83));
    assert!(
        v.get("fallback").is_none(),
        "an applied run carries no fallback: {v}"
    );
    assert_eq!(v["scores"][0]["rank"], serde_json::json!(1));
    assert_eq!(v["scores"][0]["deterministic_rank"], serde_json::json!(2));
    assert_eq!(
        v["scores"][0]["candidate_id"],
        serde_json::json!("memory:aaaa0001"),
        "the wire id is the pool key, unique within one request"
    );
    assert_eq!(
        v["scores"][0]["candidate_ref"],
        serde_json::json!("memory:aaaa0001:abcd"),
        "and the observation reference is reported beside it"
    );
    assert_eq!(v["scores"][0]["score"], serde_json::json!(0.5));
}

#[test]
fn a_declined_report_names_the_fallback_and_scores_nothing() {
    let v = serde_json::to_value(declined()).expect("serialize");
    assert_eq!(v["applied"], serde_json::json!(false));
    assert_eq!(v["adapter"], serde_json::json!("lora-v1"));
    assert_eq!(
        v["fallback"],
        serde_json::json!("reranker timed out after 300ms")
    );
    assert_eq!(v["scores"], serde_json::json!([]));
}

#[test]
fn the_summary_line_reports_the_identity_and_the_bound() {
    let line = applied().summary();
    assert!(line.contains("lexical-overlap@1"), "got: {line}");
    assert!(line.contains("prefix 2/200"), "got: {line}");
    assert!(line.contains("83ms"), "got: {line}");
    assert!(!line.contains("fallback"), "got: {line}");
}

#[test]
fn the_summary_line_reports_the_adapter_and_the_refusal() {
    let line = declined().summary();
    assert!(line.contains("(lora-v1)"), "got: {line}");
    assert!(
        line.contains("fallback: reranker timed out after 300ms"),
        "got: {line}"
    );
}
