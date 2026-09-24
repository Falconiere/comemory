#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`crate::utilities::repo_label`].

use comemory::utilities::repo_label::normalize;

#[test]
fn case_and_surrounding_whitespace_do_not_make_a_different_repository() {
    assert_eq!(normalize(" CodaSignal/Foo "), "codasignal/foo");
    assert_eq!(normalize("codasignal/foo"), "codasignal/foo");
}

#[test]
fn the_separator_and_inner_characters_are_left_alone() {
    // Only case and the edges are reduced: a label is otherwise opaque, and
    // rewriting its middle would make two distinct repositories collide.
    assert_eq!(
        normalize("Owner/Name.With-Dots_1"),
        "owner/name.with-dots_1"
    );
    assert_eq!(normalize("a b/c"), "a b/c");
}

#[test]
fn an_empty_or_blank_label_reduces_to_empty() {
    // Callers treat empty as "no label", so blanks must not survive as one.
    assert_eq!(normalize(""), "");
    assert_eq!(normalize("   "), "");
    assert_eq!(normalize("\t\n"), "");
}
