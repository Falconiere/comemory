//! Mirror tests for `src/output/search.rs`. Pins the `comemory search
//! --json` envelope contract (`score_parts` is a stable, documented
//! surface — M2 tuning reads it) via an insta snapshot, and locks in that
//! `output::search::emit` accepts an empty hit slice without panicking.

use comemory::output::search;
use comemory::retrieval::rerank::{Reranked, ScoreParts};
use comemory::retrieval::router::Source;

fn sample_hits() -> Vec<Reranked> {
    vec![Reranked {
        memory_id: "aaaa0001".into(),
        source: Source::Hybrid,
        tier: 1,
        parts: ScoreParts {
            rrf: 0.016,
            activation: 1.0,
            feedback: 1.0,
            quality: 1.0,
            supersede: 1.0,
            final_score: 0.016,
        },
        superseded_by: None,
        body: String::new(),
        simhash: 0,
    }]
}

#[test]
fn emit_accepts_empty_hits_in_json_mode() {
    // Smoke test: emitting zero hits in JSON mode must succeed. The full
    // JSON envelope shape is pinned by `search_json_envelope_contract`.
    let hits: Vec<Reranked> = Vec::new();
    search::emit(&hits, None, true).expect("emit must succeed for empty hits");
}

#[test]
fn tty_footer_omits_feedback_hint_for_empty_hits() {
    let hits: Vec<Reranked> = Vec::new();
    let mut buf: Vec<u8> = Vec::new();
    search::write_tty(&mut buf, &hits, Some("q-20260610-a1b2c3d4")).expect("write_tty");
    let out = String::from_utf8(buf).expect("utf8");
    assert!(
        out.contains("query: q-20260610-a1b2c3d4"),
        "query id footer must survive empty hits: {out}"
    );
    assert!(
        !out.contains("feedback:"),
        "no feedback hint without hits: {out}"
    );
}

#[test]
fn tty_footer_includes_feedback_hint_with_hits() {
    let hits = sample_hits();
    let mut buf: Vec<u8> = Vec::new();
    search::write_tty(&mut buf, &hits, Some("q-20260610-a1b2c3d4")).expect("write_tty");
    let out = String::from_utf8(buf).expect("utf8");
    assert!(out.contains("feedback:"), "feedback hint expected: {out}");
}

#[test]
fn search_json_envelope_contract() {
    insta::assert_json_snapshot!(search::envelope(&sample_hits(), None));
}

#[test]
fn envelope_carries_query_id_when_present() {
    let hits = sample_hits();
    let v = serde_json::to_value(search::envelope(&hits, Some("q-20260610-a1b2c3d4")))
        .expect("serialize");
    assert_eq!(
        v.get("query_id").and_then(serde_json::Value::as_str),
        Some("q-20260610-a1b2c3d4")
    );
}

#[test]
fn envelope_omits_query_id_when_absent() {
    let hits = sample_hits();
    let v = serde_json::to_value(search::envelope(&hits, None)).expect("serialize");
    assert!(
        v.get("query_id").is_none(),
        "query_id must be skipped when None: {v}"
    );
}
