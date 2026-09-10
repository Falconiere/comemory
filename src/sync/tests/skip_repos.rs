#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirrors `src/sync/skip_repos.rs` — the `[sync] skip_repos` opt-out that
//! replaced the GitHub App allowlist. Labels below are the real shapes
//! `Frontmatter.repo` carries: a bare basename, a qualified `owner/name`, a
//! hand-edited label with stray case and whitespace, and an empty one.

use comemory::sync::skip_repos::{SkipMatcher, normalize_repo_label};

#[test]
fn normalizes_case_and_surrounding_whitespace() {
    assert_eq!(normalize_repo_label("  CodaSignal/Foo  "), "codasignal/foo");
    assert_eq!(normalize_repo_label("qwick-backend"), "qwick-backend");
    assert_eq!(normalize_repo_label("   "), "");
}

#[test]
fn skip_repos_matches_normalized_labels() {
    let matcher = SkipMatcher::compile(&["acme/secret-*".to_string()]).unwrap();
    assert!(matcher.is_skipped("acme/secret-thing"));
    assert!(
        matcher.is_skipped("  Acme/Secret-Other  "),
        "a hand-edited label with stray case and padding must still be withheld"
    );
    assert!(!matcher.is_skipped("acme/public-thing"));
}

#[test]
fn skip_repos_pattern_case_is_normalized_too() {
    let matcher = SkipMatcher::compile(&["Acme/Secret-*".to_string()]).unwrap();
    assert!(matcher.is_skipped("acme/secret-thing"));
}

#[test]
fn empty_pattern_list_skips_nothing() {
    let matcher = SkipMatcher::compile(&[]).unwrap();
    assert!(!matcher.is_skipped("acme/secret-thing"));
    assert!(!matcher.is_skipped(""));
}

#[test]
fn empty_label_is_never_skipped_by_this_filter() {
    let matcher = SkipMatcher::compile(&["*".to_string()]).unwrap();
    assert!(
        !matcher.is_skipped("   "),
        "an unlabelled memory is withheld by the push filter's personal check, not here"
    );
    assert!(matcher.is_skipped("anything"));
}

#[test]
fn invalid_glob_is_rejected_by_name() {
    let err = SkipMatcher::compile(&["acme/[".to_string()]).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("skip_repos"),
        "message must name the key: {msg}"
    );
    assert!(
        msg.contains("acme/["),
        "message must name the pattern: {msg}"
    );
}
