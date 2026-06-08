//! Mirror tests for `src/output/search.rs`. The public-facing JSON envelope
//! produced by `comemory search --json` is covered end-to-end in
//! `tests/cli/search.rs`; this module exists to satisfy the tests-mirror gate
//! and to lock in that `output::search::emit` accepts an empty hit slice
//! without panicking.

use comemory::output::search;

#[test]
fn emit_accepts_empty_hits_in_json_mode() {
    // Smoke test: emitting zero hits in JSON mode must succeed. The full
    // JSON envelope shape is asserted end-to-end in `tests/cli/search.rs`
    // (`search_finds_seeded_memory_lexically`).
    let hits: Vec<comemory::retrieval::router::RoutedHit> = Vec::new();
    search::emit(&hits, true).expect("emit must succeed for empty hits");
}
