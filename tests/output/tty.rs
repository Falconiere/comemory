//! Mirror tests for `src/output/tty.rs`. The primary assertion checks that
//! `score(0.5)` produces a string containing the 3-decimal numeric form, so
//! we are robust to ANSI escapes from `owo-colors` (which only kick in when
//! the runtime detects a real TTY).

use comemory::output::tty;

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
    tty::write_query_footer(&mut buf, None, true).expect("write footer");
    assert!(buf.is_empty(), "no footer without a query id");
}

#[test]
fn query_footer_appends_hint_only_with_hits() {
    let mut with_hits: Vec<u8> = Vec::new();
    tty::write_query_footer(&mut with_hits, Some("q-20260611-a1b2c3d4"), true)
        .expect("write footer");
    let with_hits = String::from_utf8(with_hits).expect("utf8");
    assert!(with_hits.contains("query: q-20260611-a1b2c3d4"));
    assert!(with_hits.contains("feedback: comemory feedback q-20260611-a1b2c3d4"));

    let mut no_hits: Vec<u8> = Vec::new();
    tty::write_query_footer(&mut no_hits, Some("q-20260611-a1b2c3d4"), false)
        .expect("write footer");
    let no_hits = String::from_utf8(no_hits).expect("utf8");
    assert!(no_hits.contains("query: q-20260611-a1b2c3d4"));
    assert!(
        !no_hits.contains("feedback:"),
        "no feedback hint without hits: {no_hits}"
    );
}
