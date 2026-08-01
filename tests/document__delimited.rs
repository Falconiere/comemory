//! Test mirror for `src/document/delimited.rs` — CSV/TSV extraction via
//! the `csv` crate, run over a real fixture in `common/fixtures/docs/`.

use comemory::document::DocumentFormat;
use comemory::document::delimited::extract;

const DATA_CSV: &[u8] = include_bytes!("common/fixtures/docs/data.csv");

#[test]
fn title_is_always_the_file_stem() {
    let doc = extract(DATA_CSV, "data").expect("csv extract");
    assert_eq!(doc.title, "data");
    assert_eq!(doc.format, DocumentFormat::Delimited);
    assert!(
        doc.chunks.iter().all(|c| c.heading_path.is_empty()),
        "CSV has no heading structure"
    );
}

/// Concatenate every chunk's text — with 200-char overlap between forced
/// cuts, this may repeat a little content, but never drops any, so
/// `contains` checks against it are safe regardless of where a cut lands.
fn full_text(doc: &comemory::document::ExtractedDocument) -> String {
    doc.chunks
        .iter()
        .map(|c| c.text.as_str())
        .collect::<Vec<_>>()
        .join("")
}

#[test]
fn every_field_is_labeled_by_its_header() {
    let doc = extract(DATA_CSV, "data").expect("csv extract");
    let text = full_text(&doc);
    assert!(text.contains(
        "command: search, description: ranked memory search, category: retrieval, since: 0.1.0"
    ));
    assert!(text.contains(
        "command: consolidate, description: advisory near-duplicate cluster report, category: maintenance, since: 0.13.0"
    ));
    assert!(text.contains(
        "command: config, description: print the resolved layered config, category: diagnostics, since: 0.2.0"
    ));
}

#[test]
fn chunks_never_bust_the_ceiling_and_simhash_is_stable() {
    let a = extract(DATA_CSV, "data").expect("csv extract");
    let b = extract(DATA_CSV, "data").expect("csv extract");
    assert_eq!(a.chunks.len(), b.chunks.len());
    assert!(!a.chunks.is_empty());
    for (ca, cb) in a.chunks.iter().zip(b.chunks.iter()) {
        assert_eq!(ca.simhash, cb.simhash);
        assert_ne!(ca.simhash, 0);
        assert!(ca.text.chars().count() <= comemory::document::chunk::CHUNK_CHAR_CEILING);
    }
}

#[test]
fn tab_delimited_input_is_sniffed_and_parsed() {
    let tsv = b"name\tage\ncomemory\t2\nqwick\t5\n";
    let doc = extract(tsv, "roster").expect("tsv extract");
    let text = full_text(&doc);
    assert!(text.contains("name: comemory, age: 2"));
    assert!(text.contains("name: qwick, age: 5"));
}

#[test]
fn header_only_csv_yields_no_chunks() {
    let doc = extract(b"a,b,c\n", "empty").expect("csv extract");
    assert!(doc.chunks.is_empty());
}
