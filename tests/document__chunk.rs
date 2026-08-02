//! Test mirror for `src/document/chunk.rs` — size-bounded chunk splitting
//! with paragraph-boundary preference and overlap. `pack` is hit directly
//! with hand-computed offsets (pure, no I/O); `split` is exercised with
//! hand-built [`Block`]s.

use comemory::document::Block;
use comemory::document::chunk::{CHUNK_CHAR_CEILING, CHUNK_OVERLAP, pack, split};

#[test]
fn pack_empty_input_yields_no_chunks() {
    assert_eq!(pack(0, &[]), Vec::<(usize, usize)>::new());
}

#[test]
fn pack_returns_single_span_under_ceiling() {
    // Short input must not fragment at every paragraph break.
    assert_eq!(pack(500, &[0, 200, 350]), vec![(0, 500)]);
}

#[test]
fn pack_prefers_paragraph_boundary_within_ceiling() {
    let out = pack(3000, &[0, 1500]);
    assert_eq!(out[0], (0, 1500));
}

#[test]
fn pack_hard_cuts_when_no_boundary_in_window() {
    let out = pack(5000, &[0]);
    assert_eq!(out[0], (0, CHUNK_CHAR_CEILING));
}

#[test]
fn pack_carries_exact_overlap_between_forced_cuts() {
    let out = pack(5000, &[0]);
    assert_eq!(out, vec![(0, 2000), (1800, 3800), (3600, 5000)]);
    for w in out.windows(2) {
        assert_eq!(w[1].0, w[0].1 - CHUNK_OVERLAP);
        assert!(w[1].0 > w[0].0, "must make forward progress");
    }
}

#[test]
fn pack_never_reselects_a_consumed_boundary() {
    // Regression: a boundary inside the overlap window of a forced cut
    // must not be re-selected as the very next chunk's end — that would
    // emit a near-empty chunk and stall progress.
    let out = pack(4500, &[0, 1000, 2200, 4000]);
    assert_eq!(
        out,
        vec![(0, 1000), (800, 2200), (2000, 4000), (3800, 4500)]
    );
}

#[test]
fn pack_tiles_zero_to_len_within_budget() {
    let out = pack(6000, &[0, 700, 1900, 3300, 4100]);
    assert_eq!(out[0].0, 0);
    assert_eq!(out.last().unwrap().1, 6000);
    for &(s, e) in &out {
        assert!(e > s, "inverted span ({s},{e})");
        assert!(
            e - s <= CHUNK_CHAR_CEILING,
            "span ({s},{e}) busts the ceiling"
        );
    }
}

fn block(heading_path: &[&str], text: &str, char_start: usize, line_start: usize) -> Block {
    Block {
        heading_path: heading_path.iter().map(|s| s.to_string()).collect(),
        text: text.to_string(),
        char_start,
        line_start,
    }
}

#[test]
fn split_skips_blank_blocks() {
    let blocks = vec![
        block(&[], "   \n\n  ", 0, 1),
        block(&["Intro"], "Real content here.", 10, 3),
    ];
    let chunks = split(&blocks);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].text, "Real content here.");
    assert_eq!(chunks[0].heading_path, vec!["Intro".to_string()]);
    assert_eq!(chunks[0].ordinal, 0);
}

#[test]
fn split_never_spans_two_blocks_and_ordinals_are_sequential() {
    let blocks = vec![
        block(&["A"], "alpha content", 0, 1),
        block(&["B"], "beta content", 20, 2),
    ];
    let chunks = split(&blocks);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].heading_path, vec!["A".to_string()]);
    assert_eq!(chunks[1].heading_path, vec!["B".to_string()]);
    assert_eq!(chunks[0].ordinal, 0);
    assert_eq!(chunks[1].ordinal, 1);
}

#[test]
fn split_char_range_and_line_range_are_exact_for_short_text() {
    let text = "line one\nline two\n\nline four\nline five";
    let blocks = vec![block(&[], text, 0, 1)];
    let chunks = split(&blocks);
    // Well under the ceiling: one chunk covering the whole block, blank
    // line inside notwithstanding.
    assert_eq!(chunks.len(), 1);
    let c = &chunks[0];
    assert_eq!(c.char_range, (0, text.chars().count()));
    assert_eq!(c.line_range, (1, 5));
    assert_eq!(c.text, text);
}

#[test]
fn split_offsets_the_block_position_into_the_document() {
    let text = "second block\nsecond line";
    let blocks = vec![block(&["H"], text, 50, 10)];
    let chunks = split(&blocks);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].char_range, (50, 50 + text.chars().count()));
    assert_eq!(chunks[0].line_range, (10, 11));
}

#[test]
fn split_produces_exact_overlap_text_when_a_block_exceeds_the_ceiling() {
    let long_text: String = "word ".repeat(500); // 2500 chars, one paragraph
    let full_chars: Vec<char> = long_text.chars().collect();
    let blocks = vec![block(&["A"], &long_text, 0, 1)];
    let chunks = split(&blocks);

    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].char_range, (0, 2000));
    assert_eq!(chunks[1].char_range, (1800, 2500));

    let expected_overlap: String = full_chars[1800..2000].iter().collect();
    assert!(chunks[0].text.ends_with(&expected_overlap));
    assert!(chunks[1].text.starts_with(&expected_overlap));
    assert_eq!(chunks[1].heading_path, vec!["A".to_string()]);
}

#[test]
fn split_simhash_is_stable_across_runs_and_nonzero() {
    let blocks = vec![block(
        &["X"],
        "Some deterministic body text for hashing.",
        0,
        1,
    )];
    let a = split(&blocks);
    let b = split(&blocks);
    assert_eq!(a[0].simhash, b[0].simhash);
    assert_ne!(a[0].simhash, 0);
}
