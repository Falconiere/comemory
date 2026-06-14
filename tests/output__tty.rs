//! Mirror tests for `src/output/tty.rs`. The primary assertion checks that
//! `score(0.5)` produces a string containing the 3-decimal numeric form, so
//! we are robust to ANSI escapes from `owo-colors` (which only kick in when
//! the runtime detects a real TTY).

use comemory::output::tty;

/// Kill mutant `src/output/tty.rs:15`: `header` body replaced with `Ok(())`.
///
/// The original writes the header text to the output; the mutant writes
/// nothing. `write_header` (the extracted helper that `header` delegates to)
/// is called with a `Vec<u8>` buffer so the output is capturable without a
/// real TTY. The test asserts the buffer is non-empty and contains the
/// expected text — which passes on the original and fails under the mutation.
#[test]
fn write_header_emits_text_to_writer() {
    let mut buf: Vec<u8> = Vec::new();
    tty::write_header(&mut buf, "section title").expect("write_header");
    let out = String::from_utf8(buf).expect("utf8");
    assert!(
        out.contains("section title"),
        "write_header must emit the header text; got {out:?}"
    );
}

#[test]
fn score_contains_three_decimal_digits() {
    let rendered = tty::score(0.5);
    assert!(
        rendered.contains("0.500"),
        "score(0.5) should contain '0.500'; got {rendered:?}"
    );
}

#[test]
fn dim_returns_non_empty_string() {
    let rendered = tty::dim("hello");
    assert!(rendered.contains("hello"));
}

#[test]
fn query_footer_skips_when_no_query_id() {
    let mut buf: Vec<u8> = Vec::new();
    tty::write_query_footer(&mut buf, None, true, tty::FeedbackHint::Memory).expect("write footer");
    assert!(buf.is_empty(), "no footer without a query id");
}

#[test]
fn query_footer_appends_hint_only_with_hits() {
    let mut with_hits: Vec<u8> = Vec::new();
    tty::write_query_footer(
        &mut with_hits,
        Some("q-20260611-a1b2c3d4"),
        true,
        tty::FeedbackHint::Memory,
    )
    .expect("write footer");
    let with_hits = String::from_utf8(with_hits).expect("utf8");
    assert!(with_hits.contains("query: q-20260611-a1b2c3d4"));
    assert!(with_hits.contains("feedback: comemory feedback q-20260611-a1b2c3d4 --used <ids>"));
    assert!(
        !with_hits.contains("--used-code"),
        "memory flavor must not reference --used-code: {with_hits}"
    );

    let mut no_hits: Vec<u8> = Vec::new();
    tty::write_query_footer(
        &mut no_hits,
        Some("q-20260611-a1b2c3d4"),
        false,
        tty::FeedbackHint::Memory,
    )
    .expect("write footer");
    let no_hits = String::from_utf8(no_hits).expect("utf8");
    assert!(no_hits.contains("query: q-20260611-a1b2c3d4"));
    assert!(
        !no_hits.contains("feedback:"),
        "no feedback hint without hits: {no_hits}"
    );
}

#[test]
fn page_footer_shows_one_based_range() {
    // offset 4, 3 shown, 10 total -> "showing 5-7 of 10 (--offset 4)".
    let mut buf: Vec<u8> = Vec::new();
    tty::write_page_footer(&mut buf, 3, 4, Some(10)).expect("write page footer");
    let out = String::from_utf8(buf).expect("utf8");
    assert!(
        out.contains("showing 5\u{2013}7 of 10 (--offset 4)"),
        "expected 1-based range footer; got {out:?}"
    );
}

#[test]
fn page_footer_empty_shows_zero() {
    let mut buf: Vec<u8> = Vec::new();
    tty::write_page_footer(&mut buf, 0, 2, Some(5)).expect("write page footer");
    let out = String::from_utf8(buf).expect("utf8");
    assert!(
        out.contains("showing 0 of 5 (--offset 2)"),
        "empty page must show 0; got {out:?}"
    );
}

#[test]
fn page_footer_unknown_total_renders_question_mark() {
    let mut buf: Vec<u8> = Vec::new();
    tty::write_page_footer(&mut buf, 2, 0, None).expect("write page footer");
    let out = String::from_utf8(buf).expect("utf8");
    assert!(
        out.contains("showing 1\u{2013}2 of ? (--offset 0)"),
        "unknown total must render as ?; got {out:?}"
    );
}

#[test]
fn query_footer_code_flavor_references_used_code() {
    let mut buf: Vec<u8> = Vec::new();
    tty::write_query_footer(
        &mut buf,
        Some("q-20260611-a1b2c3d4"),
        true,
        tty::FeedbackHint::Code,
    )
    .expect("write footer");
    let out = String::from_utf8(buf).expect("utf8");
    assert!(
        out.contains("feedback: comemory feedback q-20260611-a1b2c3d4 --used-code <ids>"),
        "code flavor must reference --used-code: {out}"
    );
}
