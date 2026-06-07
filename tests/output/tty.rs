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
