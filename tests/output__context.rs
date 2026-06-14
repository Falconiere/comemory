//! Mirror tests for `src/output/context.rs`. The public-facing bundle shape
//! emitted by `comemory context --json` is covered end-to-end in
//! `tests/cli/context.rs`; this module pins the envelope contract (flattened
//! bundle + optional `query_id`) and locks in that `output::context::emit`
//! accepts an empty bundle without panicking.

use comemory::output::context;
use comemory::output::search::PageMeta;
use comemory::retrieval::bundle::{Bundle, CodeRow};
use comemory::retrieval::code_prior::CodePriorParts;

/// Representative memory-list pagination cursor for the unpaginated first
/// page (page size 12, no offset, empty in-window list).
fn meta() -> PageMeta {
    PageMeta {
        limit: 12,
        offset: 0,
        has_more: false,
        total: Some(0),
    }
}

fn empty_bundle() -> Bundle<'static> {
    Bundle {
        query: "smoke",
        memories: Vec::new(),
        code_refs: Vec::new(),
        relations: Vec::new(),
        resolved_code_ids: Vec::new(),
    }
}

#[test]
fn emit_accepts_empty_bundle_in_json_mode() {
    // Smoke test: emitting an empty bundle in JSON mode must succeed. The
    // full envelope shape is asserted end-to-end in `tests/cli/context.rs`
    // (`context_returns_bundle_for_seeded_memory`).
    let bundle = empty_bundle();
    context::emit(&bundle, None, meta(), true).expect("emit must succeed for empty bundle");
}

#[test]
fn envelope_carries_query_id_and_flattens_bundle() {
    let bundle = empty_bundle();
    let v = serde_json::to_value(context::envelope(
        &bundle,
        Some("q-20260611-a1b2c3d4"),
        meta(),
    ))
    .expect("serialize");
    assert_eq!(
        v.get("query_id").and_then(serde_json::Value::as_str),
        Some("q-20260611-a1b2c3d4")
    );
    // The bundle fields must stay at the top level (flattened), so existing
    // `--json` consumers keep reading `query` / `memories` unchanged.
    assert_eq!(
        v.get("query").and_then(serde_json::Value::as_str),
        Some("smoke")
    );
    assert!(v.get("memories").is_some(), "bundle fields must flatten");
}

#[test]
fn code_ref_rank_parts_serialize_when_present_and_skip_when_none() {
    let bundle = Bundle {
        query: "q",
        memories: Vec::new(),
        code_refs: vec![
            CodeRow {
                repo: "r".to_string(),
                path: "a.rs".to_string(),
                symbol: "a_run".to_string(),
                snippet: "fn a_run() {}".to_string(),
                rank_parts: Some(CodePriorParts {
                    rank: 1.2,
                    activation: 1.0,
                    affinity: 1.0,
                    feedback: 0.9,
                    final_score: 1.08,
                }),
            },
            CodeRow {
                repo: "r".to_string(),
                path: "b.rs".to_string(),
                symbol: "b_ghost".to_string(),
                snippet: String::new(),
                rank_parts: None,
            },
        ],
        relations: Vec::new(),
        resolved_code_ids: Vec::new(),
    };
    let v = serde_json::to_value(context::envelope(&bundle, None, meta())).expect("serialize");
    let refs = v["code_refs"].as_array().expect("code_refs array");
    for key in ["rank", "activation", "affinity", "feedback", "final_score"] {
        assert!(
            refs[0]["rank_parts"][key].is_number(),
            "rank_parts.{key} missing: {v}"
        );
    }
    assert!(
        refs[1].get("rank_parts").is_none(),
        "rank_parts must be skipped when None: {v}"
    );
}

#[test]
fn envelope_omits_query_id_when_absent() {
    let bundle = empty_bundle();
    let v = serde_json::to_value(context::envelope(&bundle, None, meta())).expect("serialize");
    assert!(
        v.get("query_id").is_none(),
        "query_id must be skipped when None: {v}"
    );
}
