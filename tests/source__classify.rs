//! Test mirror for `src/source/classify.rs`.

use std::path::Path;

use comemory::document::DocumentFormat;
use comemory::source::classify::{Classification, SNIFF_WINDOW, classify};

const TEXT_HEAD: &[u8] = b"# Heading\n\nSome real prose, no NUL bytes here.\n";

#[test]
fn txt_extension_classifies_as_txt() {
    let got = classify(Path::new("changelog.txt"), TEXT_HEAD);
    assert_eq!(got, Classification::Document(DocumentFormat::Txt));
}

#[test]
fn markdown_extensions_classify_as_markdown() {
    for name in ["guide.md", "guide.markdown"] {
        let got = classify(Path::new(name), TEXT_HEAD);
        assert_eq!(
            got,
            Classification::Document(DocumentFormat::Markdown),
            "{name}"
        );
    }
}

#[test]
fn html_extensions_classify_as_html() {
    for name in ["page.html", "page.htm", "page.xhtml"] {
        let got = classify(Path::new(name), TEXT_HEAD);
        assert_eq!(
            got,
            Classification::Document(DocumentFormat::Html),
            "{name}"
        );
    }
}

#[test]
fn delimited_extensions_classify_as_delimited() {
    for name in ["data.csv", "data.tsv"] {
        let got = classify(Path::new(name), TEXT_HEAD);
        assert_eq!(
            got,
            Classification::Document(DocumentFormat::Delimited),
            "{name}"
        );
    }
}

#[test]
fn extension_case_is_ignored() {
    let got = classify(Path::new("GUIDE.MD"), TEXT_HEAD);
    assert_eq!(got, Classification::Document(DocumentFormat::Markdown));
}

#[test]
fn unrecognized_extension_is_unsupported() {
    let got = classify(Path::new("archive.zip"), TEXT_HEAD);
    assert_eq!(got, Classification::Unsupported);
}

#[test]
fn no_extension_is_unsupported() {
    let got = classify(Path::new("Makefile"), TEXT_HEAD);
    assert_eq!(got, Classification::Unsupported);
}

#[test]
fn binary_content_under_allowlisted_extension_is_unsupported() {
    let binary_head: &[u8] = b"\x00\x01\x02\x03binary-not-text";
    let got = classify(Path::new("notes.md"), binary_head);
    assert_eq!(got, Classification::Unsupported);
}

#[test]
#[should_panic(expected = "classify's contract")]
fn content_head_longer_than_sniff_window_violates_the_debug_assert() {
    // classify's contract requires the caller (discover::read_head) to
    // already bound content_head to SNIFF_WINDOW; debug_assert! catches
    // a caller that doesn't, rather than silently clamping forever.
    let head = vec![b'a'; SNIFF_WINDOW + 1];
    let _ = classify(Path::new("notes.txt"), &head);
}
