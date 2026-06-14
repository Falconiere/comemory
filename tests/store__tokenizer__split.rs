use comemory::store::tokenizer::split::{SplitToken, fold_diacritics, split_text};
use proptest::prelude::*;

fn texts(tokens: &[SplitToken]) -> Vec<&str> {
    tokens.iter().map(|t| t.text.as_str()).collect()
}

#[test]
fn camel_case_splits_with_colocated_whole() {
    let toks = split_text("VecDimMismatch");
    assert_eq!(
        texts(&toks),
        vec!["vec", "vecdimmismatch", "dim", "mismatch"]
    );
    assert!(!toks[0].colocated);
    assert!(toks[1].colocated);
    assert!(!toks[2].colocated);
    assert!(!toks[3].colocated);
}

#[test]
fn snake_case_splits() {
    let toks = split_text("parse_html");
    assert_eq!(texts(&toks), vec!["parse", "parse_html", "html"]);
}

#[test]
fn digit_boundaries_split() {
    let toks = split_text("sha256sum");
    assert_eq!(texts(&toks), vec!["sha", "sha256sum", "256", "sum"]);
}

#[test]
fn plain_words_emit_single_token() {
    let toks = split_text("hello world");
    assert_eq!(texts(&toks), vec!["hello", "world"]);
    assert!(toks.iter().all(|t| !t.colocated));
}

#[test]
fn byte_offsets_point_into_original_text() {
    let text = "use VecDim now";
    for t in split_text(text) {
        if t.text == "vecdim" || t.text == "vec" || t.text == "dim" {
            assert!(
                t.start >= 4 && t.end <= 10,
                "bad offsets {}..{}",
                t.start,
                t.end
            );
        }
    }
}

#[test]
fn acronym_runs_stay_grouped() {
    let toks = split_text("HTMLParser");
    assert_eq!(texts(&toks), vec!["html", "htmlparser", "parser"]);
}

#[test]
fn leading_underscore_is_trimmed() {
    let toks = split_text("_foo");
    assert_eq!(texts(&toks), vec!["foo"]);
}

#[test]
fn dunder_wrapped_word_emits_inner_part() {
    let toks = split_text("__init__");
    assert_eq!(texts(&toks), vec!["init"]);
}

#[test]
fn underscore_only_runs_emit_nothing() {
    let toks = split_text("a __ b");
    assert_eq!(texts(&toks), vec!["a", "b"]);
}

#[test]
fn non_ascii_words_are_tokenized_and_diacritic_folded() {
    // Diacritics fold so `café` and `cafe` index/query identically —
    // restores the old `unicode61 remove_diacritics 2` behavior.
    let toks = split_text("über café");
    assert_eq!(texts(&toks), vec!["uber", "cafe"]);
    assert!(toks.iter().all(|t| !t.colocated));
}

#[test]
fn diacritics_fold_to_ascii_base_letters() {
    assert_eq!(texts(&split_text("café")), vec!["cafe"]);
    assert_eq!(texts(&split_text("naïve")), vec!["naive"]);
    assert_eq!(texts(&split_text("über")), vec!["uber"]);
    // Already-folded text is untouched.
    assert_eq!(texts(&split_text("cafe")), vec!["cafe"]);
    // NFD input (base letter + combining mark) folds to the same token as
    // the precomposed form.
    assert_eq!(texts(&split_text("cafe\u{301}")), vec!["cafe"]);
    assert_eq!(fold_diacritics("éàü"), "eau");
}

#[test]
fn unlowercaseable_chars_never_emit_uppercase_tokens() {
    // U+1F130 SQUARED LATIN CAPITAL LETTER A: is_uppercase() is true but
    // to_lowercase() is a no-op, so any token containing it is dropped.
    let toks = split_text("🄰");
    assert!(
        toks.iter()
            .all(|t| t.text.chars().all(|c| !c.is_uppercase()))
    );
}

proptest! {
    #[test]
    fn never_panics_and_tokens_are_lowercase(s in "\\PC*") {
        for t in split_text(&s) {
            prop_assert!(t.text.chars().all(|c| !c.is_uppercase()));
            prop_assert!(t.end <= s.len());
            prop_assert!(t.start <= t.end);
            if !t.colocated {
                prop_assert_eq!(
                    fold_diacritics(&s[t.start..t.end].to_lowercase()),
                    t.text
                );
            }
        }
    }

    #[test]
    fn first_token_of_a_run_is_never_colocated(s in "\\PC*") {
        let toks = split_text(&s);
        if let Some(first) = toks.first() {
            prop_assert!(!first.colocated);
        }
    }
}
