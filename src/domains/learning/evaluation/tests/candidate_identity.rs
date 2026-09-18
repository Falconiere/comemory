#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Coverage for the candidate-ref codec: the string form is the opaque
//! domain-qualified token the reranker process protocol passes around, so it
//! has to round-trip every real identity shape — including repo labels, paths
//! and symbol names carrying the two reserved characters — and reject a
//! malformed reference loudly instead of accepting a wrong candidate.

use comemory::domains::learning::evaluation::candidate_identity::{
    CandidateDomain, CandidateIdentity, CodeIdentity, DocumentIdentity, MemoryIdentity, parse_ref,
};

/// A real-shaped memory identity: an 8-hex id and its 64-hex body digest.
fn memory() -> CandidateIdentity {
    CandidateIdentity::Memory(MemoryIdentity {
        memory_id: "5a9f19bc".into(),
        content_hash: "5a9f19bc403e9aa0752a777b56073d1aed3a367fc78d25d9631b0be5ccaeb702".into(),
    })
}

/// A real-shaped code identity, path and all.
fn code() -> CandidateIdentity {
    CandidateIdentity::Code(CodeIdentity {
        repo: "comemory".into(),
        path: "src/domains/retrieval/rerank.rs".into(),
        symbol: "rerank".into(),
        blob_oid: "9d1c1f0b8a4e2f6c5b3a7d9e0f1a2b3c4d5e6f70".into(),
    })
}

/// A real-shaped document identity: a 32-hex parent id and a chunk ordinal.
fn document() -> CandidateIdentity {
    CandidateIdentity::Document(DocumentIdentity {
        document_id: "0f1e2d3c4b5a69788796a5b4c3d2e1f0".into(),
        path: "guides/schema-migrations.md".into(),
        revision_hash: "7b2a9c4e".into(),
        chunk_ordinal: 3,
    })
}

#[test]
fn every_domain_round_trips_through_its_reference_string() {
    for identity in [memory(), code(), document()] {
        let raw = identity.candidate_ref();
        let back = parse_ref(&raw).expect("a reference this codec produced must parse");
        assert_eq!(back, identity, "round trip must be exact for {raw}");
        assert_eq!(back.candidate_ref(), raw, "encoding must be stable");
    }
}

#[test]
fn a_path_keeps_its_slashes_literal_and_the_domain_leads() {
    let raw = code().candidate_ref();
    assert!(
        raw.starts_with("code:comemory:src/domains/retrieval/rerank.rs:rerank:"),
        "slashes are not in the escaped set, so a path reads as a path: {raw}"
    );
}

#[test]
fn reserved_characters_in_identity_components_survive_the_round_trip() {
    let identity = CandidateIdentity::Code(CodeIdentity {
        // A colon would otherwise split into an extra component, and a percent
        // would otherwise be read as the start of an escape.
        repo: "host:8080/group".into(),
        path: "src/a%b:c.rs".into(),
        symbol: "Mod::method".into(),
        blob_oid: "deadbeef".into(),
    });
    let raw = identity.candidate_ref();
    assert_eq!(
        raw.matches(':').count(),
        4,
        "only the four separators may remain literal in {raw}"
    );
    assert_eq!(parse_ref(&raw).expect("round trip"), identity);
}

#[test]
fn multi_byte_text_in_a_symbol_round_trips_byte_exactly() {
    let identity = CandidateIdentity::Code(CodeIdentity {
        repo: "café".into(),
        path: "src/日本語.rs".into(),
        symbol: "naïve:fn".into(),
        blob_oid: "cafebabe".into(),
    });
    assert_eq!(
        parse_ref(&identity.candidate_ref()).expect("round trip"),
        identity
    );
}

#[test]
fn content_version_reports_the_field_a_judgment_pin_is_compared_against() {
    assert_eq!(memory().content_version().len(), 64);
    assert_eq!(
        code().content_version(),
        "9d1c1f0b8a4e2f6c5b3a7d9e0f1a2b3c4d5e6f70"
    );
    assert_eq!(document().content_version(), "7b2a9c4e");
}

#[test]
fn domain_labels_match_the_fusion_vocabulary() {
    assert_eq!(CandidateDomain::Memory.as_str(), "memory");
    assert_eq!(CandidateDomain::Code.as_str(), "code");
    assert_eq!(CandidateDomain::Document.as_str(), "document");
    assert_eq!(memory().domain(), CandidateDomain::Memory);
}

#[test]
fn a_malformed_reference_errors_naming_the_reference() {
    let cases = [
        ("memory:only-one", "expected 2 components"),
        ("memory:a:b:c", "expected 2 components"),
        ("video:a:b", "unknown domain `video`"),
        ("", "unknown domain ``"),
        ("document:a:b:c:notanumber", "is not an integer"),
        ("document:a:b:c", "expected 4 components"),
        ("code:a:b%ZZ:c:d", "is not a hex digit"),
        ("code:a:b%4:c:d", "truncated `%` escape"),
        ("code:a:%FF:c:d", "not valid UTF-8"),
    ];
    for (raw, expected) in cases {
        let err = parse_ref(raw).expect_err("must not accept a malformed reference");
        let msg = err.to_string();
        assert!(
            msg.contains(expected),
            "error for {raw:?} should say {expected:?}, said {msg:?}"
        );
        assert!(
            msg.contains(raw) || raw.is_empty(),
            "error for {raw:?} must quote the reference, said {msg:?}"
        );
    }
}

#[test]
fn identity_serializes_with_a_domain_tag_and_deserializes_back() {
    let json = serde_json::to_string(&document()).expect("serialize");
    assert!(json.contains("\"domain\":\"document\""), "{json}");
    let back: CandidateIdentity = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, document(), "the artifact must round-trip (AC-11)");
}
