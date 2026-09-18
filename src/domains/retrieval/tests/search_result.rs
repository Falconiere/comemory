#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/domains/retrieval/search_result.rs`.
//!
//! The `--json` envelope this module builds is pinned end to end by the
//! snapshot mirrors at `tests/output__search.rs` (memory) and
//! `tests/output__search_code.rs` (code), which is where AGENTS.md puts
//! snapshot tests. What those cannot reach is every arm of `source_label`:
//! a snapshot only ever records the branch its fixture produced, so the
//! `Vector` and `Graph` labels have no assertion anywhere. This pins all
//! four — they are the `source` vocabulary both `--json` envelopes and both
//! TTY writers share, so a relabel is a contract break.

use comemory::retrieval::router::Source;
use comemory::retrieval::search_result::source_label;

#[test]
fn every_retrieval_source_has_its_stable_lowercase_label() {
    assert_eq!(source_label(Source::Vector), "vector");
    assert_eq!(source_label(Source::Lexical), "lexical");
    assert_eq!(source_label(Source::Hybrid), "hybrid");
    assert_eq!(source_label(Source::Graph), "graph");
}
