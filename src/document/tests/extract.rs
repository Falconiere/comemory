#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/document/extract.rs` — dispatch plus the
//! TXT/Markdown extractors, run over real fixture files in
//! `common/fixtures/docs/`.

use comemory::document::DocumentFormat;
use comemory::document::extract::extract;

const GUIDE_MD: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/common/fixtures/docs/guide.md"
));
const CHANGELOG_TXT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/common/fixtures/docs/changelog.txt"
));

fn path(strs: &[&str]) -> Vec<String> {
    strs.iter().map(std::string::ToString::to_string).collect()
}

#[test]
fn txt_extraction_uses_file_stem_as_title_and_keeps_one_block() {
    let doc = extract(DocumentFormat::Txt, CHANGELOG_TXT, "changelog").expect("txt extract");
    assert_eq!(doc.title, "changelog");
    assert_eq!(doc.format, DocumentFormat::Txt);
    assert_eq!(
        doc.chunks.len(),
        1,
        "723-byte file must stay one chunk under the 2000-char ceiling"
    );
    assert!(doc.chunks[0].heading_path.is_empty());
    assert_eq!(doc.chunks[0].text, String::from_utf8_lossy(CHANGELOG_TXT));
}

#[test]
fn markdown_title_is_the_first_heading() {
    let doc = extract(DocumentFormat::Markdown, GUIDE_MD, "guide").expect("markdown extract");
    assert_eq!(doc.title, "Comemory CLI Guide");
    assert_eq!(doc.format, DocumentFormat::Markdown);
}

#[test]
fn markdown_heading_path_breadcrumb_tracks_nesting_exactly() {
    // One chunk per heading section (each section is well under the
    // 2000-char ceiling), in document order, breadcrumb outermost-first.
    let doc = extract(DocumentFormat::Markdown, GUIDE_MD, "guide").expect("markdown extract");
    let paths: Vec<Vec<String>> = doc.chunks.iter().map(|c| c.heading_path.clone()).collect();
    assert_eq!(
        paths,
        vec![
            path(&["Comemory CLI Guide"]),
            path(&["Comemory CLI Guide", "Installation"]),
            path(&["Comemory CLI Guide", "Installation", "Homebrew"]),
            path(&["Comemory CLI Guide", "Indexing a repository"]),
            path(&[
                "Comemory CLI Guide",
                "Indexing a repository",
                "Supported languages"
            ]),
            path(&[
                "Comemory CLI Guide",
                "Indexing a repository",
                "Ignoring paths"
            ]),
            path(&["Comemory CLI Guide", "Searching"]),
            path(&["Comemory CLI Guide", "Searching", "Memory search"]),
            path(&["Comemory CLI Guide", "Searching", "Code search"]),
            path(&["Comemory CLI Guide", "Troubleshooting"]),
            path(&["Comemory CLI Guide", "Next steps"]),
        ],
    );
    for (i, c) in doc.chunks.iter().enumerate() {
        assert_eq!(c.ordinal, i);
    }
}

#[test]
fn markdown_heading_inside_a_fence_is_not_treated_as_a_heading() {
    // "Ignoring paths" fences a line starting with '#' (a bash comment).
    // If fence-tracking failed, that line would be misdetected as an h1,
    // resetting the whole breadcrumb stack and blowing up the section
    // count / paths asserted above.
    let doc = extract(DocumentFormat::Markdown, GUIDE_MD, "guide").expect("markdown extract");
    let ignoring_paths = doc
        .chunks
        .iter()
        .find(|c| c.heading_path.last().map(String::as_str) == Some("Ignoring paths"))
        .expect("Ignoring paths section present");
    assert!(ignoring_paths.text.contains("# build artifacts"));
    assert_eq!(
        doc.chunks.len(),
        11,
        "fence-guard failure would fragment/duplicate sections"
    );
}

#[test]
fn markdown_chunks_never_bust_the_ceiling_and_simhash_is_stable() {
    let a = extract(DocumentFormat::Markdown, GUIDE_MD, "guide").expect("markdown extract");
    let b = extract(DocumentFormat::Markdown, GUIDE_MD, "guide").expect("markdown extract");
    assert_eq!(a.chunks.len(), b.chunks.len());
    for (ca, cb) in a.chunks.iter().zip(b.chunks.iter()) {
        assert_eq!(ca.simhash, cb.simhash);
        assert_ne!(ca.simhash, 0);
        assert!(ca.text.chars().count() <= comemory::document::chunk::CHUNK_CHAR_CEILING);
    }
}

#[test]
fn markdown_with_no_headings_falls_back_to_file_stem_title() {
    let doc = extract(
        DocumentFormat::Markdown,
        b"just a paragraph, no headings here.\n",
        "notes",
    )
    .expect("markdown extract");
    assert_eq!(doc.title, "notes");
    assert_eq!(doc.chunks.len(), 1);
    assert!(doc.chunks[0].heading_path.is_empty());
}
