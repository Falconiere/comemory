//! Mirror tests for `src/output/search_code.rs`. Pins the `comemory
//! search-code --json` envelope contract (`lines` is a 2-element
//! `[start, end]` array; `score_parts` is the stable explainability
//! surface) via an insta snapshot, and locks in the TTY shape:
//! `score path:start-end symbol (kind) #id` rows, the code-flavored
//! `--used-code` feedback footer, and the empty-index hint.

use comemory::output::search::PageMeta;
use comemory::output::search_code;
use comemory::retrieval::code_rerank::{CodeReranked, CodeScoreParts};
use comemory::retrieval::router::Source;

/// Representative pagination cursor for the unpaginated first page: page
/// size 12 (default `top_k`), no offset, single in-window hit.
fn meta() -> PageMeta {
    PageMeta {
        limit: 12,
        offset: 0,
        has_more: false,
        total: Some(1),
    }
}

fn sample_hits() -> Vec<CodeReranked> {
    vec![CodeReranked {
        symbol_id: 12,
        repo: "r".into(),
        path: "src/lib.rs".into(),
        symbol: "alpha_router".into(),
        kind: "function".into(),
        lang: "rust".into(),
        line_start: 3,
        line_end: 9,
        source: Source::Lexical,
        parts: CodeScoreParts {
            relevance: 1.0,
            rank: 1.0,
            activation: 1.0,
            affinity: 1.0,
            feedback: 1.0,
            final_score: 1.0,
        },
    }]
}

#[test]
fn emit_accepts_empty_hits_in_json_mode() {
    // Smoke test: emitting zero hits in JSON mode must succeed. The full
    // envelope shape is pinned by `search_code_json_envelope_contract`.
    let hits: Vec<CodeReranked> = Vec::new();
    search_code::emit(&hits, None, meta(), false, true).expect("emit must succeed for empty hits");
}

#[test]
fn search_code_json_envelope_contract() {
    insta::assert_json_snapshot!(search_code::envelope(&sample_hits(), None, meta()));
}

#[test]
fn envelope_carries_query_id_when_present() {
    let v = serde_json::to_value(search_code::envelope(
        &sample_hits(),
        Some("q-20260611-a1b2c3d4"),
        meta(),
    ))
    .expect("serialize");
    assert_eq!(
        v.get("query_id").and_then(serde_json::Value::as_str),
        Some("q-20260611-a1b2c3d4")
    );
}

#[test]
fn envelope_omits_query_id_when_absent() {
    let v = serde_json::to_value(search_code::envelope(&sample_hits(), None, meta()))
        .expect("serialize");
    assert!(
        v.get("query_id").is_none(),
        "query_id must be skipped when None: {v}"
    );
}

#[test]
fn envelope_serializes_lines_as_start_end_array() {
    let v = serde_json::to_value(search_code::envelope(&sample_hits(), None, meta()))
        .expect("serialize");
    assert_eq!(
        v["hits"][0]["lines"],
        serde_json::json!([3, 9]),
        "lines must serialize as a [start, end] array: {v}"
    );
}

#[test]
fn tty_row_shows_score_path_lines_symbol_kind_and_id() {
    let mut buf: Vec<u8> = Vec::new();
    search_code::write_tty(&mut buf, &sample_hits(), Some("q-20260611-a1b2c3d4"), false)
        .expect("write_tty");
    let out = String::from_utf8(buf).expect("utf8");
    let row = out.lines().next().expect("hit row");
    assert!(row.contains("1.000"), "score: {row}");
    assert!(row.contains("src/lib.rs:3-9"), "path:start-end: {row}");
    assert!(row.contains("alpha_router"), "symbol: {row}");
    assert!(row.contains("(function)"), "kind: {row}");
    assert!(row.contains("#12"), "feedback-able symbol id: {row}");
    assert!(
        out.contains("feedback: comemory feedback q-20260611-a1b2c3d4 --used-code"),
        "code feedback hint: {out}"
    );
}

#[test]
fn tty_footer_omits_feedback_hint_for_empty_hits() {
    let hits: Vec<CodeReranked> = Vec::new();
    let mut buf: Vec<u8> = Vec::new();
    search_code::write_tty(&mut buf, &hits, Some("q-20260611-a1b2c3d4"), false).expect("write_tty");
    let out = String::from_utf8(buf).expect("utf8");
    assert!(
        out.contains("query: q-20260611-a1b2c3d4"),
        "query id footer must survive empty hits: {out}"
    );
    assert!(
        !out.contains("feedback:"),
        "no feedback hint without hits: {out}"
    );
    assert!(
        !out.contains("index-code"),
        "populated index must not hint index-code: {out}"
    );
}

#[test]
fn tty_empty_index_prints_index_code_hint() {
    let hits: Vec<CodeReranked> = Vec::new();
    let mut buf: Vec<u8> = Vec::new();
    search_code::write_tty(&mut buf, &hits, None, true).expect("write_tty");
    let out = String::from_utf8(buf).expect("utf8");
    assert!(
        out.contains("comemory index-code"),
        "empty index must hint comemory index-code: {out}"
    );
}
