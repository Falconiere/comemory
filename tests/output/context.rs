//! Mirror tests for `src/output/context.rs`. The public-facing bundle shape
//! emitted by `comemory context --json` is covered end-to-end in
//! `tests/cli/context.rs`; this module pins the envelope contract (flattened
//! bundle + optional `query_id`) and locks in that `output::context::emit`
//! accepts an empty bundle without panicking.

use comemory::output::context;
use comemory::retrieval::bundle::Bundle;

fn empty_bundle() -> Bundle<'static> {
    Bundle {
        query: "smoke",
        memories: Vec::new(),
        code_refs: Vec::new(),
        relations: Vec::new(),
    }
}

#[test]
fn emit_accepts_empty_bundle_in_json_mode() {
    // Smoke test: emitting an empty bundle in JSON mode must succeed. The
    // full envelope shape is asserted end-to-end in `tests/cli/context.rs`
    // (`context_returns_bundle_for_seeded_memory`).
    let bundle = empty_bundle();
    context::emit(&bundle, None, true).expect("emit must succeed for empty bundle");
}

#[test]
fn envelope_carries_query_id_and_flattens_bundle() {
    let bundle = empty_bundle();
    let v = serde_json::to_value(context::envelope(&bundle, Some("q-20260611-a1b2c3d4")))
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
fn envelope_omits_query_id_when_absent() {
    let bundle = empty_bundle();
    let v = serde_json::to_value(context::envelope(&bundle, None)).expect("serialize");
    assert!(
        v.get("query_id").is_none(),
        "query_id must be skipped when None: {v}"
    );
}
