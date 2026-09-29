#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/domains/projects/evidence.rs`: an unrecognised stored
//! trust reads as `invalid` and the `invalid` filter keeps it; each claim
//! shape the platform refuses is refused with its field and reason, and each
//! well-shaped claim earns the initial trust of the platform's matrix.

use comemory::domains::projects::evidence::{
    Claim, Initial, PENDING_REASON, read_trust, trust_filter, view,
};
use comemory::errors::Error;
use comemory::store::project_evidence::{EvidenceRow, TrustFilter};
use comemory::utilities::project_error::ProjectError;
use serde_json::json;

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

fn claim<'a>(
    kind: &'a str,
    repo: Option<&'a str>,
    sha: Option<&'a str>,
    external: Option<&'a str>,
) -> Claim<'a> {
    Claim {
        kind,
        repo,
        commit_sha: sha,
        external_id: external,
    }
}

/// The `(field, reason)` of a `422 invalid_request`.
fn refused(result: comemory::errors::Result<()>) -> (String, String) {
    let Err(Error::Project(ProjectError::InvalidRequest { details, .. })) = result else {
        panic!("expected invalid_request, got {result:?}");
    };
    let details = serde_json::to_value(details).unwrap();
    (
        details["field"].as_str().unwrap().to_string(),
        details["reason"].as_str().unwrap().to_string(),
    )
}

#[test]
fn an_unrecognised_stored_trust_reads_as_the_least_trusted_state() {
    for known in ["verified", "self_reported", "pending", "invalid"] {
        assert_eq!(read_trust(known), known);
    }
    assert_eq!(read_trust("bogus"), "invalid");
    assert_eq!(read_trust(""), "invalid");
    assert!(matches!(
        trust_filter("pending"),
        TrustFilter::Exactly("pending")
    ));
    let TrustFilter::NoneOf(kept_out) = trust_filter("invalid") else {
        panic!("invalid must keep every unrecognised value");
    };
    assert_eq!(kept_out, ["verified", "self_reported", "pending"]);
}

#[test]
fn claims_the_platform_cannot_verify_are_refused_with_their_reason() {
    let cases = [
        (
            claim("commit", None, Some(SHA), None),
            ("repo", "required_for_commit"),
        ),
        (
            claim("commit", Some("a/b"), None, None),
            ("commitSha", "required_hex_sha"),
        ),
        (
            claim("pull_request", None, None, Some("7")),
            ("repo", "required_for_pull_request"),
        ),
        (
            claim("pull_request", Some("a/b"), None, Some("seven")),
            ("externalId", "required_pull_request_number"),
        ),
        (
            claim("pull_request", Some("a/b"), None, Some("0")),
            ("externalId", "required_pull_request_number"),
        ),
        (
            claim("test_run", Some("a/b"), None, Some("ci")),
            ("commitSha", "required_hex_sha"),
        ),
        (
            claim("test_run", Some("a/b"), Some(SHA), None),
            ("externalId", "required_check_run_name"),
        ),
        (
            claim("memory", None, None, None),
            ("externalId", "required_record_id"),
        ),
        (
            claim("session", None, None, None),
            ("externalId", "required_record_id"),
        ),
        (
            claim("decision", None, None, None),
            ("externalId", "required_record_id"),
        ),
    ];
    for (claim, (field, reason)) in cases {
        assert_eq!(
            refused(claim.check_shape()),
            (field.to_string(), reason.to_string()),
            "{claim:?}"
        );
    }
}

#[test]
fn well_shaped_claims_earn_the_platform_matrix_trust() {
    let pending = Initial {
        trust: "pending",
        reason: Some(PENDING_REASON),
    };
    let self_reported = Initial {
        trust: "self_reported",
        reason: None,
    };
    let cases = [
        (claim("commit", Some("a/b"), Some(SHA), None), pending),
        (claim("pull_request", Some("a/b"), None, Some("7")), pending),
        (
            claim("test_run", Some("a/b"), Some(SHA), Some("ci")),
            pending,
        ),
        (claim("test_run", None, None, None), self_reported),
        (claim("memory", None, None, Some("ab12cd34")), pending),
        (claim("session", None, None, Some("s-1")), pending),
        (claim("decision", None, None, Some("d-1")), pending),
        (claim("deployment", None, None, None), self_reported),
        (claim("external_url", None, None, None), self_reported),
    ];
    for (claim, expected) in cases {
        claim.check_shape().unwrap();
        assert_eq!(claim.initial(), expected, "{claim:?}");
    }
}

#[test]
fn the_view_reads_unparsable_metadata_as_empty_and_unknown_trust_as_invalid() {
    let row = EvidenceRow {
        id: "e0000000-0000-4000-8000-000000000001".into(),
        work_item_id: None,
        execution_id: None,
        kind: "commit".into(),
        source: "git".into(),
        external_id: None,
        url: None,
        trust: "bogus".into(),
        metadata: "[1, 2]".into(),
        content_hash: None,
        verified_by: Some("engine".into()),
        verified_at: Some(0),
        creator_principal_type: "user".into(),
        creator_principal_id: "local-operator".into(),
        created_at: 1_790_000_000_000,
    };
    let shown = serde_json::to_value(view(row).unwrap()).unwrap();
    assert_eq!(shown["trust"], "invalid");
    assert_eq!(shown["metadata"], json!({}));
    assert_eq!(shown["verifiedAt"], "1970-01-01T00:00:00.000Z");
    assert_eq!(shown["createdAt"], "2026-09-21T14:13:20.000Z");
}
