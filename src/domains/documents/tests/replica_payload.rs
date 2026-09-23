#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The document revision payload, built from a REAL extraction of this
//! repository's own `docs/guides/cloud-sync.md` through the production
//! extractor — not a hand-written fixture.

use std::path::Path;

use crate::domains::documents::document::{DocumentFormat, extract};
use crate::domains::documents::replica_payload::{
    ChunkWire, DOCUMENT_ENTITY_KIND, DOCUMENT_PAYLOAD_VERSION, DocumentRevisionV1, LinkWire,
};
use crate::domains::documents::share;

const REPO: &str = "Falconiere/comemory";
const PATH: &str = "docs/guides/cloud-sync.md";

/// A payload over the real extracted chunks of a real repository document.
fn real_revision() -> DocumentRevisionV1 {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join(PATH);
    let body = std::fs::read(&file).unwrap_or_else(|e| panic!("read real {PATH}: {e}"));
    let extracted = extract::extract(DocumentFormat::Markdown, &body, "cloud-sync")
        .expect("the production extractor");
    assert!(
        extracted.chunks.len() > 1,
        "the fixture document must chunk into several passages, got {}",
        extracted.chunks.len()
    );
    let chunks: Vec<ChunkWire> = extracted
        .chunks
        .iter()
        .map(|c| ChunkWire {
            ordinal: c.ordinal as i64,
            heading_path: c.heading_path.join(" > "),
            char_start: c.char_range.0 as i64,
            char_end: c.char_range.1 as i64,
            line_start: c.line_range.0 as i64,
            line_end: c.line_range.1 as i64,
            simhash: c.simhash as i64,
            text: c.text.clone(),
        })
        .collect();
    DocumentRevisionV1 {
        shared_id: share::shared_id(REPO, PATH),
        repo: REPO.to_string(),
        path: PATH.to_string(),
        title: extracted.title.clone(),
        format: "markdown".to_string(),
        revision_hash: "d".repeat(64),
        chunks,
        links: vec![LinkWire {
            ordinal: 0,
            target: "docs/guides/http-api.md".to_string(),
        }],
    }
}

#[test]
fn the_kind_and_version_are_the_ones_the_wire_declares() {
    assert_eq!(DOCUMENT_ENTITY_KIND, "document_revision");
    assert_eq!(DOCUMENT_PAYLOAD_VERSION, 1);
}

#[test]
fn canonical_bytes_round_trip_and_carry_a_stable_digest() {
    let revision = real_revision();

    let (bytes, digest) = revision.canonical().expect("canonical");
    let decoded = DocumentRevisionV1::decode(&bytes).expect("decode");

    assert_eq!(decoded, revision, "the bytes are the payload, losslessly");
    assert_eq!(
        revision.canonical().expect("again").1,
        digest,
        "and the digest is a function of the contents only"
    );
    assert_eq!(digest.len(), 64);
}

#[test]
fn the_payload_owns_the_id_its_repo_and_path_earn() {
    let revision = real_revision();

    assert!(revision.owns_its_id(), "{}", revision.shared_id);
    assert_eq!(revision.mint_id(), share::shared_id(REPO, PATH));
}

#[test]
fn relabelling_a_document_costs_it_its_id() {
    let revision = real_revision();

    // Same extracted text, someone else's repository.
    let moved_repo = DocumentRevisionV1 {
        repo: "Falconiere/other".to_string(),
        ..revision.clone()
    };
    assert!(
        !moved_repo.owns_its_id(),
        "a document cannot keep its name under a different repository"
    );

    // Same extracted text, a different path in the same repository.
    let moved_path = DocumentRevisionV1 {
        path: "docs/guides/http-api.md".to_string(),
        ..revision.clone()
    };
    assert!(!moved_path.owns_its_id(), "nor under a different path");

    // An id nobody earned.
    let forged = DocumentRevisionV1 {
        shared_id: "f".repeat(32),
        ..revision.clone()
    };
    assert!(!forged.owns_its_id());

    // A short id is not a valid one either.
    let truncated = DocumentRevisionV1 {
        shared_id: "abc".to_string(),
        ..revision
    };
    assert!(!truncated.owns_its_id());
}

#[test]
fn two_revisions_of_one_document_share_its_id() {
    let first = real_revision();
    let second = DocumentRevisionV1 {
        revision_hash: "e".repeat(64),
        ..first.clone()
    };

    assert_eq!(
        first.shared_id, second.shared_id,
        "the id names WHICH document, not which revision — that is what lets \
         the later one supersede the earlier"
    );
    assert_ne!(
        first.canonical().expect("first").1,
        second.canonical().expect("second").1,
        "but their bytes differ, so the feed positions do too"
    );
}

#[test]
fn a_gap_in_the_chunk_ordinals_is_visible() {
    let revision = real_revision();
    assert!(
        revision.chunks_are_contiguous(),
        "the real extraction is contiguous"
    );

    let mut holed = revision.clone();
    holed.chunks.remove(1);
    assert!(
        !holed.chunks_are_contiguous(),
        "a missing middle passage must be detectable, not published"
    );

    let mut repeated = revision;
    repeated.chunks[1].ordinal = 0;
    assert!(!repeated.chunks_are_contiguous());
}

#[test]
fn a_link_pointing_past_the_last_chunk_is_visible() {
    let revision = real_revision();
    assert!(revision.links_resolve());

    let mut dangling = revision.clone();
    dangling.links[0].ordinal = i64::try_from(revision.chunks.len()).expect("fits");
    assert!(!dangling.links_resolve(), "a link into nothing");

    let mut negative = revision;
    negative.links[0].ordinal = -1;
    assert!(!negative.links_resolve());
}

#[test]
fn an_unknown_field_is_refused_rather_than_ignored() {
    let revision = real_revision();
    let (bytes, _) = revision.canonical().expect("canonical");
    let mut value: serde_json::Value = serde_json::from_str(&bytes).expect("json");
    value["canonical_path"] = serde_json::json!("/Users/someone/checkout/docs/x.md");

    let refused = DocumentRevisionV1::decode(&value.to_string())
        .expect_err("an unknown field could be a machine path smuggled in");

    assert!(
        matches!(&refused, crate::errors::Error::BadRequest(m) if m.contains("document revision")),
        "got {refused:?}"
    );
}

#[test]
fn nothing_on_the_wire_is_an_absolute_path() {
    let revision = real_revision();
    let (bytes, _) = revision.canonical().expect("canonical");

    // The real document's own repository checkout, which must not appear.
    let checkout = env!("CARGO_MANIFEST_DIR");
    assert!(
        !bytes.contains(checkout),
        "the payload leaked this machine's checkout path"
    );
    assert!(
        !bytes.contains("\"canonical_path\""),
        "the payload has no field for an absolute path"
    );
    assert!(
        bytes.contains("docs/guides/cloud-sync.md"),
        "but it does carry the repository-relative one"
    );
}
