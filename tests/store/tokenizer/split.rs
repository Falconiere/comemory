use comemory::store::tokenizer::split::{split_text, SplitToken};
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

proptest! {
    #[test]
    fn never_panics_and_tokens_are_lowercase(s in "\\PC*") {
        for t in split_text(&s) {
            prop_assert!(t.text.chars().all(|c| !c.is_uppercase()));
            prop_assert!(t.end <= s.len());
            prop_assert!(t.start <= t.end);
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
