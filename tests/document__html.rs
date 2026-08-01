//! Test mirror for `src/document/html.rs` — HTML/XHTML extraction via
//! `tl`, run over a real fixture page in `common/fixtures/docs/`.

use comemory::document::DocumentFormat;
use comemory::document::html::extract;

const PAGE_HTML: &[u8] = include_bytes!("common/fixtures/docs/page.html");

fn path(strs: &[&str]) -> Vec<String> {
    strs.iter().map(|s| s.to_string()).collect()
}

#[test]
fn title_comes_from_the_title_tag_not_the_first_heading() {
    let doc = extract(PAGE_HTML, "page").expect("html extract");
    assert_eq!(doc.title, "Comemory — Local-First Dev Memory");
    assert_eq!(doc.format, DocumentFormat::Html);
}

#[test]
fn heading_path_breadcrumb_tracks_nesting_exactly() {
    let doc = extract(PAGE_HTML, "page").expect("html extract");
    let paths: Vec<Vec<String>> = doc.chunks.iter().map(|c| c.heading_path.clone()).collect();
    assert_eq!(
        paths,
        vec![
            path(&["Comemory"]),
            path(&["Comemory", "Installation"]),
            path(&["Comemory", "Installation", "Homebrew"]),
            path(&["Comemory", "Installation", "From source"]),
            path(&["Comemory", "Usage"]),
            path(&["Comemory", "Usage", "Indexing"]),
            path(&["Comemory", "Usage", "Searching"]),
        ],
    );
    for (i, c) in doc.chunks.iter().enumerate() {
        assert_eq!(c.ordinal, i);
    }
}

#[test]
fn script_and_style_content_never_reaches_a_chunk() {
    let doc = extract(PAGE_HTML, "page").expect("html extract");
    for c in &doc.chunks {
        assert!(
            !c.text.contains("console.log"),
            "script content leaked: {:?}",
            c.text
        );
        assert!(
            !c.text.contains("trackClick"),
            "script content leaked: {:?}",
            c.text
        );
        assert!(
            !c.text.contains("font-family"),
            "style content leaked: {:?}",
            c.text
        );
        assert!(
            !c.text.contains(".hero"),
            "style content leaked: {:?}",
            c.text
        );
    }
}

#[test]
fn body_text_is_extracted_under_the_right_heading() {
    let doc = extract(PAGE_HTML, "page").expect("html extract");
    let homebrew = doc
        .chunks
        .iter()
        .find(|c| c.heading_path.last().map(String::as_str) == Some("Homebrew"))
        .expect("Homebrew section present");
    assert!(
        homebrew
            .text
            .contains("brew install Falconiere/tap/comemory")
    );

    let searching = doc
        .chunks
        .iter()
        .find(|c| c.heading_path.last().map(String::as_str) == Some("Searching"))
        .expect("Searching section present");
    assert!(searching.text.contains("comemory search-code"));
}

#[test]
fn chunks_never_bust_the_ceiling_and_simhash_is_stable() {
    let a = extract(PAGE_HTML, "page").expect("html extract");
    let b = extract(PAGE_HTML, "page").expect("html extract");
    assert_eq!(a.chunks.len(), b.chunks.len());
    for (ca, cb) in a.chunks.iter().zip(b.chunks.iter()) {
        assert_eq!(ca.simhash, cb.simhash);
        assert_ne!(ca.simhash, 0);
        assert!(ca.text.chars().count() <= comemory::document::chunk::CHUNK_CHAR_CEILING);
    }
}

#[test]
fn html_with_no_title_or_headings_falls_back_to_file_stem() {
    let doc =
        extract(b"<html><body><p>hello world</p></body></html>", "untitled").expect("html extract");
    assert_eq!(doc.title, "untitled");
}

#[test]
fn html_title_tag_wins_over_a_present_heading() {
    let doc = extract(
        b"<html><head><title>Real Title</title></head><body><h1>Heading</h1></body></html>",
        "stem",
    )
    .expect("html extract");
    assert_eq!(doc.title, "Real Title");
}
