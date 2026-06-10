//! Mirror tests for `src/output/search.rs`. Pins the `comemory search
//! --json` envelope contract (`score_parts` is a stable, documented
//! surface — M2 tuning reads it) via an insta snapshot, and locks in that
//! `output::search::emit` accepts an empty hit slice without panicking.

use comemory::output::search;
use comemory::retrieval::rerank::{Reranked, ScoreParts};
use comemory::retrieval::router::Source;

#[test]
fn emit_accepts_empty_hits_in_json_mode() {
    // Smoke test: emitting zero hits in JSON mode must succeed. The full
    // JSON envelope shape is pinned by `search_json_envelope_contract`.
    let hits: Vec<Reranked> = Vec::new();
    search::emit(&hits, true).expect("emit must succeed for empty hits");
}

#[test]
fn search_json_envelope_contract() {
    let hits = vec![Reranked {
        memory_id: "aaaa0001".into(),
        source: Source::Hybrid,
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
    }];
    insta::assert_json_snapshot!(search::envelope(&hits));
}
