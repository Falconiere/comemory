//! Test mirror for `src/ast/chunk.rs` — cAST-style greedy chunking of
//! oversized AST nodes.
//!
//! The end-to-end tests run a REAL oversized function (copied from this
//! repo into `fixtures/oversized_fn.rs`) through the real extractor; the
//! property tests hit the pure packing function directly.

use comemory::ast::chunk::{pack_spans, Chunk, CHUNK_LINE_BUDGET, MIN_CHUNK_LINES};
use comemory::ast::{extract, Lang};
use proptest::prelude::*;

const OVERSIZED_SRC: &str = include_str!("fixtures/oversized_fn.rs");

/// Extract the oversized fixture symbol's chunks through the real
/// extractor (not a hand-built node) so the test covers the full
/// `extract` → `chunk_node` wiring.
fn oversized_chunks() -> Vec<Chunk> {
    let syms = extract(Lang::Rust, OVERSIZED_SRC).expect("extract oversized fixture");
    let sym = syms
        .iter()
        .find(|s| s.name == "with_env")
        .expect("with_env symbol extracted");
    sym.chunks.clone()
}

#[test]
fn oversized_fn_splits_at_ast_boundaries_within_budget() {
    let chunks = oversized_chunks();
    assert!(chunks.len() >= 2, "expected >= 2 chunks, got {chunks:?}");
    for c in &chunks {
        assert!(
            c.line_end - c.line_start < CHUNK_LINE_BUDGET || c.single_node,
            "chunk over budget without single-node excuse: {c:?}",
        );
        assert!(c.line_end >= c.line_start, "inverted span: {c:?}");
    }
    for w in chunks.windows(2) {
        assert!(
            w[1].line_start > w[0].line_end,
            "chunks must not overlap: {:?} then {:?}",
            w[0],
            w[1],
        );
    }
}

#[test]
fn oversized_fn_chunks_tile_body_below_signature() {
    // The signature line stays out of the chunks (the parent row keeps
    // it); the chunk sequence starts on the line after the symbol's first
    // line and ends on the symbol's last line.
    let syms = extract(Lang::Rust, OVERSIZED_SRC).expect("extract oversized fixture");
    let sym = syms
        .iter()
        .find(|s| s.name == "with_env")
        .expect("with_env symbol extracted");
    let snippet_lines = sym.snippet.lines().count();
    let sym_last_line = sym.line + snippet_lines - 1;
    let first = sym.chunks.first().expect("at least one chunk");
    let last = sym.chunks.last().expect("at least one chunk");
    assert_eq!(
        first.line_start,
        sym.line + 1,
        "chunks start after signature"
    );
    assert_eq!(
        last.line_end, sym_last_line,
        "chunks end at the symbol's last line"
    );
}

#[test]
fn chunk_text_is_sliced_by_its_line_span() {
    let lines: Vec<&str> = OVERSIZED_SRC.lines().collect();
    for c in &oversized_chunks() {
        let expected = lines[c.line_start - 1..c.line_end].join("\n");
        assert_eq!(c.text, expected, "chunk text must match its source lines");
    }
}

#[test]
fn small_symbol_yields_no_chunks() {
    let src = include_str!("fixtures/small_fn.rs");
    let syms = extract(Lang::Rust, src).expect("extract small fixture");
    assert!(!syms.is_empty(), "small fixture extracts symbols");
    for s in &syms {
        assert!(
            s.chunks.is_empty(),
            "symbol under budget must stay unchunked: {s:?}",
        );
    }
}

/// Generate sorted, non-overlapping one-based line spans: each entry is a
/// (gap-before, length) pair folded into absolute coordinates.
fn span_seq() -> impl Strategy<Value = Vec<(usize, usize)>> {
    proptest::collection::vec((0usize..4, 1usize..120), 1..40).prop_map(|raw| {
        let mut spans = Vec::with_capacity(raw.len());
        let mut next = 1usize;
        for (gap, len) in raw {
            let s = next + gap;
            let e = s + len - 1;
            spans.push((s, e));
            next = e + 1;
        }
        spans
    })
}

proptest! {
    #[test]
    fn pack_spans_never_overlaps_and_tiles_endpoints(spans in span_seq()) {
        let out = pack_spans(&spans);
        prop_assert!(!out.is_empty());
        // Coverage endpoints: first chunk starts at the first child's
        // start, last chunk ends at the last child's end.
        prop_assert_eq!(out[0].0, spans[0].0);
        prop_assert_eq!(out[out.len() - 1].1, spans[spans.len() - 1].1);
        for w in out.windows(2) {
            prop_assert!(w[1].0 > w[0].1, "chunks overlap: {:?} then {:?}", w[0], w[1]);
        }
    }

    #[test]
    fn pack_spans_respects_budget_unless_single_node(spans in span_seq()) {
        let out = pack_spans(&spans);
        for &(s, e, single) in &out {
            prop_assert!(e >= s, "inverted chunk ({s},{e})");
            let width = e - s + 1;
            prop_assert!(
                width <= CHUNK_LINE_BUDGET || single,
                "chunk ({s},{e}) over budget without single-node excuse",
            );
            if single {
                // A single-node chunk is exactly one oversized input span,
                // emitted verbatim so the node layer can recurse into it.
                prop_assert!(width > CHUNK_LINE_BUDGET);
                prop_assert!(spans.contains(&(s, e)), "single-node chunk not an input span");
            }
        }
    }

    #[test]
    fn pack_spans_merges_small_trailing_fragments(spans in span_seq()) {
        let out = pack_spans(&spans);
        if out.len() < 2 {
            return Ok(()); // a lone chunk may be arbitrarily small
        }
        let last = out[out.len() - 1];
        let prev = out[out.len() - 2];
        let last_width = last.1 - last.0 + 1;
        if last_width < MIN_CHUNK_LINES && !last.2 {
            // A sub-minimum trailing fragment may only survive when the
            // merge target is a pristine single-node chunk (kept verbatim
            // for recursion) or the merge would have busted the budget.
            prop_assert!(
                prev.2 || (last.1 - prev.0 + 1) > CHUNK_LINE_BUDGET,
                "trailing fragment {last:?} not merged into {prev:?}",
            );
        }
    }
}
