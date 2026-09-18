#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Coverage for judgment targets: a target that could never match must fail to
//! resolve naming the task, and a version pin that disagrees must downgrade a
//! hit to `Stale` rather than count as a match.

use comemory::domains::learning::evaluation::candidate_identity::{
    CandidateDomain, CandidateIdentity, CodeIdentity, DocumentIdentity, MemoryIdentity,
};
use comemory::domains::learning::evaluation::judgment::{
    Judgment, JudgmentTarget, MatchOutcome, TargetKey,
};

/// Parse one judgment out of YAML, exactly as a set file carries it.
fn parse(yaml: &str) -> Judgment {
    serde_yaml::from_str(yaml).expect("judgment yaml")
}

/// An observed memory identity.
fn memory(id: &str, hash: &str) -> CandidateIdentity {
    CandidateIdentity::Memory(MemoryIdentity {
        memory_id: id.into(),
        content_hash: hash.into(),
    })
}

#[test]
fn a_memory_target_resolves_from_its_yaml_shape() {
    let j = parse("relevance: 3\ntarget:\n  domain: memory\n  id: 5a9f19bc\n");
    assert_eq!(j.relevance, 3);
    assert_eq!(
        j.target.resolve("t1").expect("resolve"),
        TargetKey::Memory {
            id: "5a9f19bc".into(),
            content_hash: None
        }
    );
}

#[test]
fn a_code_target_requires_the_whole_identity_triple() {
    let j = parse("relevance: 2\ntarget:\n  domain: code\n  repo: demo\n  path: src/a.rs\n");
    let err = j.target.resolve("code-01").expect_err("symbol is required");
    let msg = err.to_string();
    assert!(msg.contains("code-01"), "must name the task: {msg}");
    assert!(msg.contains("`symbol`"), "must name the key: {msg}");
}

#[test]
fn a_key_from_another_domain_is_refused_rather_than_ignored() {
    let j = parse("relevance: 1\ntarget:\n  domain: memory\n  id: 5a9f19bc\n  symbol: rerank\n");
    let err = j
        .target
        .resolve("mem-01")
        .expect_err("symbol is not a memory key");
    let msg = err.to_string();
    assert!(msg.contains("mem-01") && msg.contains("`symbol`"), "{msg}");
    assert!(
        msg.contains("could never match"),
        "the message must say why it is refused: {msg}"
    );
}

#[test]
fn an_unknown_key_is_refused_by_deny_unknown_fields() {
    let err = serde_yaml::from_str::<Judgment>(
        "relevance: 1\ntarget:\n  domain: memory\n  id: 5a9f19bc\n  typo: x\n",
    )
    .expect_err("an unknown key must not be silently dropped");
    assert!(err.to_string().contains("typo"), "{err}");
}

#[test]
fn an_unpinned_target_matches_any_content_version() {
    let key = TargetKey::Memory {
        id: "5a9f19bc".into(),
        content_hash: None,
    };
    assert_eq!(key.matches(&memory("5a9f19bc", "aaaa")), MatchOutcome::Yes);
    assert_eq!(key.matches(&memory("5a9f19bc", "bbbb")), MatchOutcome::Yes);
    assert_eq!(key.matches(&memory("76a8b36c", "aaaa")), MatchOutcome::No);
}

#[test]
fn a_pinned_target_meeting_a_different_version_is_stale_not_a_match() {
    let key = TargetKey::Memory {
        id: "5a9f19bc".into(),
        content_hash: Some("aaaa".into()),
    };
    assert_eq!(key.matches(&memory("5a9f19bc", "aaaa")), MatchOutcome::Yes);
    assert_eq!(
        key.matches(&memory("5a9f19bc", "bbbb")),
        MatchOutcome::Stale,
        "the body moved under the judgment; scoring it as a hit would be a lie"
    );
    assert_eq!(
        key.matches(&memory("76a8b36c", "aaaa")),
        MatchOutcome::No,
        "a different memory is not stale, it is simply not this one"
    );
}

#[test]
fn a_code_target_matches_the_identity_triple_and_pins_the_blob() {
    let observed = CandidateIdentity::Code(CodeIdentity {
        repo: "demo".into(),
        path: "src/a.rs".into(),
        symbol: "alpha".into(),
        blob_oid: "0f1e".into(),
    });
    let key = TargetKey::Code {
        repo: "demo".into(),
        path: "src/a.rs".into(),
        symbol: "alpha".into(),
        blob_oid: None,
    };
    assert_eq!(key.matches(&observed), MatchOutcome::Yes);
    let pinned = TargetKey::Code {
        repo: "demo".into(),
        path: "src/a.rs".into(),
        symbol: "alpha".into(),
        blob_oid: Some("4b5a".into()),
    };
    assert_eq!(pinned.matches(&observed), MatchOutcome::Stale);
}

#[test]
fn a_document_target_matches_on_the_identity_path_and_can_pin_the_chunk() {
    let observed = CandidateIdentity::Document(DocumentIdentity {
        document_id: "0f1e2d3c4b5a69788796a5b4c3d2e1f0".into(),
        path: "guides/chunking.md".into(),
        revision_hash: "7b2a".into(),
        chunk_ordinal: 3,
    });
    let key = TargetKey::Document {
        path: "guides/chunking.md".into(),
        revision_hash: None,
        chunk_ordinal: None,
    };
    assert_eq!(key.matches(&observed), MatchOutcome::Yes);
    let other_chunk = TargetKey::Document {
        path: "guides/chunking.md".into(),
        revision_hash: None,
        chunk_ordinal: Some(1),
    };
    assert_eq!(other_chunk.matches(&observed), MatchOutcome::Stale);
    let other_doc = TargetKey::Document {
        path: "guides/other.md".into(),
        revision_hash: None,
        chunk_ordinal: None,
    };
    assert_eq!(other_doc.matches(&observed), MatchOutcome::No);
}

#[test]
fn a_target_never_matches_across_domains() {
    let key = TargetKey::Memory {
        id: "5a9f19bc".into(),
        content_hash: None,
    };
    let code = CandidateIdentity::Code(CodeIdentity {
        repo: "demo".into(),
        path: "5a9f19bc".into(),
        symbol: "5a9f19bc".into(),
        blob_oid: "0f1e".into(),
    });
    assert_eq!(key.matches(&code), MatchOutcome::No);
    assert_eq!(key.domain(), CandidateDomain::Memory);
}

#[test]
fn a_document_target_rejects_a_repo_key() {
    let j = parse("relevance: 1\ntarget:\n  domain: document\n  path: a.md\n  repo: demo\n");
    let err = j
        .target
        .resolve("doc-01")
        .expect_err("repo is not a document key");
    assert!(err.to_string().contains("`repo`"), "{err}");
}

#[test]
fn a_blank_required_key_is_treated_as_absent() {
    let j = parse("relevance: 1\ntarget:\n  domain: memory\n  id: \"   \"\n");
    let err = j
        .target
        .resolve("mem-02")
        .expect_err("whitespace is not an id");
    assert!(err.to_string().contains("requires `id`"), "{err}");
}

#[test]
fn a_target_round_trips_through_json_for_the_unmatched_report() {
    let j =
        parse("relevance: 2\ntarget:\n  domain: code\n  repo: demo\n  path: a.rs\n  symbol: f\n");
    let json = serde_json::to_string(&j.target).expect("serialize");
    let back: JudgmentTarget = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(
        back.resolve("t").expect("resolve"),
        j.target.resolve("t").expect("resolve")
    );
}
