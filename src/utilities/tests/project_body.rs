#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/utilities/project_body.rs`: each malformed body names
//! its field the way the platform's `invalidRequestFrom` does, always at the
//! schema edge (`400`) — including a proposal's operation list past its
//! 2,000-element schema bound and a patch carrying an identity key (named by
//! its operation index).

use comemory::domains::projects::create;
use comemory::domains::projects::operations::{Operation, bounded};
use comemory::errors::{Error, Result};
use comemory::utilities::error_code::{Class, classify};
use comemory::utilities::project_body::body;
use serde::Deserialize;
use serde_json::json;

/// `(message, details)` of a schema-edge refusal.
fn refusal<T: std::fmt::Debug>(result: Result<T>) -> (String, String) {
    let e = result.expect_err("expected a refusal");
    assert_eq!(classify(&e), ("invalid_request", Class::BadRequest), "{e}");
    let Error::Project(project) = &e else {
        panic!("{e:?}")
    };
    (
        e.to_string(),
        serde_json::to_string(&project.details()).unwrap(),
    )
}

#[test]
fn body_refusals_name_the_field() {
    let cases: [(&str, &str, &str); 8] = [
        (
            "[]",
            "body must be an object",
            r#"{"field":"body","reason":"invalid"}"#,
        ),
        (
            "not json",
            "body must be an object",
            r#"{"field":"body","reason":"invalid"}"#,
        ),
        (
            r#"{"idempotencyKey":"k","keyPrefix":"AB","outcome":"o"}"#,
            "name is required",
            r#"{"field":"name","reason":"required"}"#,
        ),
        (
            r#"{"idempotencyKey":"k","name":5,"keyPrefix":"AB","outcome":"o"}"#,
            "name is invalid",
            r#"{"field":"name","reason":"invalid"}"#,
        ),
        (
            r#"{"idempotencyKey":"k","name":"n","keyPrefix":"AB","outcome":"o","successCriteria":["a",7]}"#,
            "successCriteria.1 is invalid",
            r#"{"field":"successCriteria.1","reason":"invalid"}"#,
        ),
        (
            r#"{"name":"n","keyPrefix":"AB","outcome":"o"}"#,
            "idempotencyKey is required",
            r#"{"field":"idempotencyKey","reason":"required"}"#,
        ),
        (
            r#"{"name":"n","keyPrefix":"AB","outcome":"o","idempotencyKey":7}"#,
            "idempotencyKey is invalid",
            r#"{"field":"idempotencyKey","reason":"invalid"}"#,
        ),
        (
            r#"{"name":"n","keyPrefix":"AB","outcome":"o","idempotencyKey":"k","extra":1}"#,
            "extra is invalid",
            r#"{"field":"extra","reason":"invalid"}"#,
        ),
    ];
    for (raw, message, details) in cases {
        let got = refusal(body::<create::Request>(raw.as_bytes()));
        assert_eq!(
            (got.0.as_str(), got.1.as_str()),
            (message, details),
            "{raw}"
        );
    }
    let ok: create::Request = body(
        br#"{"workspaceId":"ws","idempotencyKey":"k","name":"n","keyPrefix":"AB","outcome":"o"}"#,
    )
    .unwrap();
    assert_eq!(ok.name, "n");
}

/// A proposal-shaped body: the bounded list, as the submit request holds it.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct Proposal {
    #[serde(deserialize_with = "bounded")]
    operations: Vec<Operation>,
}

#[test]
fn a_proposal_body_past_the_schema_bound_or_patching_an_identity_names_its_field() {
    let id = "d0000000-0000-4000-8000-000000000001";
    let archive = json!({"op": "criterion.archive", "criterionId": id});
    let admitted = json!({"operations": vec![archive.clone(); 2000]}).to_string();
    let parsed: Proposal = body(admitted.as_bytes()).unwrap();
    assert_eq!(parsed.operations.len(), 2000);
    let over = json!({"operations": vec![archive.clone(); 2001]}).to_string();
    let got = refusal(body::<Proposal>(over.as_bytes()));
    assert_eq!(got.1, r#"{"field":"operations","reason":"invalid"}"#);
    let patch = json!({"operations": [archive,
        {"op": "work_item.update", "workItemId": id, "patch": {"id": id}}]})
    .to_string();
    let got = refusal(body::<Proposal>(patch.as_bytes()));
    // An internally tagged enum buffers its content, so the path stops at the
    // operation: the refusal names which operation, not the key inside it.
    assert_eq!(got.1, r#"{"field":"operations.1","reason":"invalid"}"#);
}
