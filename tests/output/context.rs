//! Mirror tests for `src/output/context.rs`. The public-facing bundle shape
//! emitted by `comemory context --json` is covered end-to-end in
//! `tests/cli/context.rs`; this module exists to satisfy the tests-mirror
//! gate and to lock in that `output::context::emit` accepts an empty bundle
//! without panicking.

use comemory::output::context;
use comemory::retrieval::bundle::Bundle;

#[test]
fn emit_accepts_empty_bundle_in_json_mode() {
    // Smoke test: emitting an empty bundle in JSON mode must succeed. The
    // full envelope shape is asserted end-to-end in `tests/cli/context.rs`
    // (`context_returns_bundle_for_seeded_memory`).
    let bundle = Bundle {
        query: "smoke",
        memories: Vec::new(),
        code_refs: Vec::new(),
        relations: Vec::new(),
    };
    context::emit(&bundle, true).expect("emit must succeed for empty bundle");
}
