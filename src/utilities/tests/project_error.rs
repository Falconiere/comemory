#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/utilities/project_error.rs`: the platform's message
//! and ordered `details` per code, the schema-edge constructor, and the
//! module doc's recorded count. The cross-adapter table (HTTP, CLI exit,
//! MCP) and the platform byte fixture live in
//! `tests/utilities__project_error.rs`.

use comemory::utilities::project_error::{ProjectError, RequestEdge};

/// `details` serialized, for byte comparison.
fn details_json(e: &ProjectError) -> String {
    serde_json::to_string(&e.details()).unwrap()
}

/// `completion_requirements_unmet` always carries all four lists, empty ones
/// as `[]`, and `criterionIds` is unmet then unverified.
#[test]
fn completion_details_carry_every_list_in_platform_order() {
    let e = ProjectError::CompletionRequirementsUnmet {
        unmet_criterion_ids: vec!["crit_a".into()],
        unverified_criterion_ids: vec![],
        work_item_ids: vec![],
    };
    assert_eq!(
        details_json(&e),
        r#"{"code":"completion_requirements_unmet","criterionIds":["crit_a"],"unmetCriterionIds":["crit_a"],"unverifiedCriterionIds":[],"workItemIds":[]}"#
    );
    assert_eq!(
        e.to_string(),
        "This project still has unmet completion requirements"
    );
}

/// A bare refusal is `{code}`; a caller-reason refusal's message is the
/// reason, and the reason never enters `details`.
#[test]
fn bare_and_reason_refusals_carry_only_their_code() {
    let reason = ProjectError::InvalidTransition {
        reason: "the project is archived".into(),
    };
    assert_eq!(reason.to_string(), "the project is archived");
    assert_eq!(details_json(&reason), r#"{"code":"invalid_transition"}"#);
    assert_eq!(
        details_json(&ProjectError::ExecutionActorForbidden),
        r#"{"code":"forbidden"}"#
    );
    assert_eq!(
        details_json(&ProjectError::Invariant {
            invariant: "plan_operation_kind".into(),
            message: "unknown plan operation".into(),
        }),
        r#"{"code":"internal_error","invariant":"plan_operation_kind"}"#
    );
}

/// `invalid_field` is the platform's `invalidRequestFrom`: message
/// `"<field> is <reason>"`, details `{field, reason}` with no `code`.
#[test]
fn invalid_field_matches_the_platform_schema_edge() {
    let e = ProjectError::invalid_field("cursor", "invalid");
    assert_eq!(e.to_string(), "cursor is invalid");
    assert_eq!(details_json(&e), r#"{"field":"cursor","reason":"invalid"}"#);
    assert!(matches!(
        e,
        ProjectError::InvalidRequest {
            edge: RequestEdge::Schema,
            ..
        }
    ));
}

/// The module doc records the real count and why the epic's first count
/// was corrected (issue #322 acceptance).
#[test]
fn module_doc_records_twenty_two_and_the_eighteen_correction() {
    let source = include_str!("../project_error.rs");
    let doc: String = source
        .lines()
        .take_while(|line| line.starts_with("//!"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(doc.contains("Twenty-two codes"), "{doc}");
    assert!(doc.contains("said eighteen"), "{doc}");
    for missed in [
        "execution_not_found",
        "evidence_not_found",
        "forbidden",
        "internal_error",
    ] {
        assert!(doc.contains(missed), "doc must name {missed}");
    }
}
